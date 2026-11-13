use super::{consts::*, regex_adapter as re};
use crate::common::{Diagnostic, diag};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
pub type EvidenceMap = BTreeMap<String, (String, String)>;
pub type SkillMap = BTreeMap<String, Value>;
pub fn arr(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
pub fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}
pub fn ss(v: &Value) -> Vec<String> {
    arr(v)
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}
pub fn set(v: &Value) -> BTreeSet<String> {
    ss(v).into_iter().collect()
}
pub fn ne(v: &Value) -> bool {
    v.as_str().is_some_and(|x| !x.trim().is_empty())
}
pub fn sl(v: &Value, empty: bool) -> bool {
    v.as_array()
        .is_some_and(|a| (empty || !a.is_empty()) && a.iter().all(ne))
}
pub fn exact(v: &Value, fields: &[&str]) -> bool {
    v.as_object()
        .is_some_and(|m| m.len() == fields.len() && fields.iter().all(|k| m.contains_key(*k)))
}
pub fn flat(v: &Value) -> Vec<String> {
    match v {
        Value::String(x) => vec![x.clone()],
        Value::Array(a) => a.iter().flat_map(flat).collect(),
        Value::Object(m) => m.values().flat_map(flat).collect(),
        _ => Vec::new(),
    }
}
pub fn substantive(v: &Value) -> bool {
    match v {
        Value::String(x) => !x.trim().is_empty(),
        Value::Array(a) => {
            !a.is_empty()
                && a.iter().all(|i| {
                    (i.is_string() || i.is_object())
                        && !flat(i).is_empty()
                        && flat(i).iter().all(|s| !s.trim().is_empty())
                })
        }
        _ => false,
    }
}
pub fn fixed(v: &Value) -> Vec<String> {
    arr(v)
        .iter()
        .filter(|i| i.is_object() && ne(&i["id"]) && ne(&i["version"]) && ne(&i["content"]))
        .map(|i| s(&i["content"]).to_owned())
        .collect()
}
thread_local! {static REGEX_ERRORS:std::cell::RefCell<Vec<String>>=const{std::cell::RefCell::new(Vec::new())};}
pub fn regex_error(error: impl std::fmt::Display) {
    REGEX_ERRORS.with(|e| e.borrow_mut().push(error.to_string()))
}
pub fn finish(mut d: Vec<Diagnostic>) -> Vec<Diagnostic> {
    REGEX_ERRORS.with(|e| {
        for error in e.borrow_mut().drain(..) {
            diag(&mut d, "QUALITY_REGEX_UNRESOLVED", error)
        }
    });
    crate::common::sorted(d)
}
pub fn has(pattern: &str, text: &str) -> bool {
    match re::matching(pattern, text) {
        Ok(v) => v,
        Err(e) => {
            regex_error(e);
            false
        }
    }
}
pub fn full(pattern: &str, text: &str) -> bool {
    match re::full(pattern, text) {
        Ok(v) => v,
        Err(e) => {
            regex_error(e);
            false
        }
    }
}
pub fn finds(pattern: &str, text: &str) -> Vec<re::Capture> {
    match re::captures(pattern, text) {
        Ok(v) => v,
        Err(e) => {
            regex_error(e);
            Vec::new()
        }
    }
}
pub fn replace(pattern: &str, text: &str, replacement: &str) -> String {
    match re::replace(pattern, text, replacement) {
        Ok(v) => v,
        Err(e) => {
            regex_error(e);
            String::new()
        }
    }
}
pub fn words(pattern: &str, text: &str) -> Vec<String> {
    match re::strings(pattern, text) {
        Ok(v) => v,
        Err(e) => {
            regex_error(e);
            Vec::new()
        }
    }
}
pub fn rstrip(text: &str, chars: &str) -> String {
    text.chars()
        .rev()
        .skip_while(|c| chars.contains(*c))
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}
pub fn safe(value: &str) -> bool {
    !value.trim().is_empty()
        && value != "."
        && value != "./"
        && !value.starts_with(['/', '\\'])
        && !value.contains('\\')
        && !value.contains('\0')
        && !has(r"^[A-Za-z]:", value)
        && !value.split('/').any(|p| p == "..")
}
pub fn confined(target: &Path, value: &str) -> Option<PathBuf> {
    if !safe(value) {
        return None;
    }
    let root = infra_core::fs::canonicalize(target).ok()?;
    let mut path = root.clone();
    for part in Path::new(value).components() {
        if let std::path::Component::Normal(name) = part {
            path.push(name);
            if path.is_symlink() {
                return None;
            }
        }
    }
    let mut tail = Vec::new();
    let mut parent = path.as_path();
    while !parent.exists() {
        tail.push(parent.file_name()?.to_owned());
        parent = parent.parent()?;
    }
    let mut resolved = infra_core::fs::canonicalize(parent).ok()?;
    if !resolved.starts_with(&root) {
        return None;
    }
    for piece in tail.iter().rev() {
        resolved.push(piece)
    }
    Some(resolved)
}
pub fn load(path: &Path, d: &mut Vec<Diagnostic>) -> Value {
    match crate::common::read_object(path) {
        Ok(v) => v,
        Err(e) => {
            diag(
                d,
                "PLAN_INVALID",
                format!("cannot read plan {}: {e}", path.display()),
            );
            Value::Null
        }
    }
}
pub fn check_ref(
    d: &mut Vec<Diagnostic>,
    code: &str,
    name: &str,
    refs: &Value,
    known: &BTreeSet<String>,
) {
    for item in ss(refs) {
        if !known.contains(&item) {
            diag(d, code, format!("{name} references unknown id: {item}"))
        }
    }
}
pub fn ids(v: &Value) -> BTreeSet<String> {
    arr(v)
        .iter()
        .filter(|i| ne(&i["id"]))
        .map(|i| s(&i["id"]).trim().to_owned())
        .collect()
}
pub fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
pub fn typed(schema: &str) -> bool {
    TYPED_PLAN_SCHEMAS.contains(&schema)
}
pub fn wired(schema: &str) -> bool {
    WIRED_PLAN_SCHEMAS.contains(&schema)
}
pub fn disposed(schema: &str) -> bool {
    DISPOSED_PLAN_SCHEMAS.contains(&schema)
}
pub fn split(pattern: &str, text: &str) -> Vec<String> {
    match re::split(pattern, text) {
        Ok(v) => v,
        Err(e) => {
            regex_error(e);
            Vec::new()
        }
    }
}
pub fn py_lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut at = 0;
    let mut cursor = text.char_indices().peekable();
    while let Some((i, c)) = cursor.next() {
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
            out.push(&text[at..i]);
            at = i + c.len_utf8();
            if c == '\r' && cursor.peek().is_some_and(|(_, c)| *c == '\n') {
                let (i, c) = cursor.next().unwrap();
                at = i + c.len_utf8();
            }
        }
    }
    if at < text.len() {
        out.push(&text[at..])
    }
    out
}
pub fn universal_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}
pub fn read_text(path: impl AsRef<Path>) -> std::io::Result<String> {
    std::fs::read_to_string(path).map(|s| universal_newlines(&s))
}
pub fn decode_lossy(bytes: &[u8]) -> String {
    universal_newlines(&String::from_utf8_lossy(bytes))
}
pub fn decode_ignore(mut bytes: &[u8]) -> String {
    let mut text = String::new();
    while !bytes.is_empty() {
        match std::str::from_utf8(bytes) {
            Ok(valid) => {
                text.push_str(valid);
                break;
            }
            Err(error) => {
                let len = error.valid_up_to();
                if let Ok(valid) = std::str::from_utf8(&bytes[..len]) {
                    text.push_str(valid)
                }
                let Some(skip) = error.error_len() else { break };
                bytes = &bytes[len + skip..];
            }
        }
    }
    universal_newlines(&text)
}
