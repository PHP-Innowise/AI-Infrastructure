//! Gate 1: a capsule's structure and dataflow, checked without running anything.
//!
//! The gate reads a candidate as the compiler returned it: the capsule, the
//! package files and the observed values. It reports every violation it can
//! see in one pass, each with a stable code, so one repair round can answer
//! all of them. Gate 1T is the lighter check of the textual form.
use crate::capsule::{
    BINDING_BYTES, BindingOrigin, BindingType, CapsuleKind, Examples, Form, SkillCapsule, Step,
    Violation, is_admitted_value, is_relative_path,
};
use crate::compile_rules::{compile_bounded_binding_regex, value_within_binding_limit};
use crate::package::Files;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// The written procedure of a textual capsule is meant to be shorter than the
/// skill it replaces.
pub const MARKDOWN_BYTES: usize = 24 * 1024;
/// A literal this long is specific enough to be an observed value rather than
/// a word that happens to occur in a description.
const EXAMPLE_LITERAL_CHARS: usize = 8;

/// What a candidate is checked against besides itself.
#[derive(Default)]
pub struct Context<'a> {
    /// Contents of the files the observed run read. A binding that equals one
    /// is file content, not a parameter.
    pub observed_files: &'a [String],
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    pub gate: &'static str,
    pub passed: bool,
    pub violations: Vec<Violation>,
}
impl Report {
    fn of(gate: &'static str, violations: Vec<Violation>) -> Self {
        Self {
            gate,
            passed: violations.is_empty(),
            violations,
        }
    }
    pub fn has(&self, code: &str) -> bool {
        self.violations
            .iter()
            .any(|violation| violation.code == code)
    }
}

/// Interpreter switches that take the program as an argument instead of a file.
const INLINE_CODE: &[&str] = &[
    "-c", "-e", "-E", "--eval", "-p", "--print", "-r", "-n", "-pe", "-ne", "-Command", "/c", "/C",
    "/k", "/K",
];

fn is_disallowed_wrapper_runtime(runtime: &str) -> bool {
    matches!(
        runtime.to_ascii_lowercase().as_str(),
        "env"
            | "sudo"
            | "doas"
            | "xargs"
            | "nohup"
            | "nice"
            | "time"
            | "timeout"
            | "stdbuf"
            | "setsid"
            | "chroot"
            | "su"
            | "script"
            | "expect"
    )
}

fn is_shell_runtime(runtime: &str) -> bool {
    matches!(
        runtime.to_ascii_lowercase().as_str(),
        "sh" | "bash" | "dash" | "zsh" | "ksh" | "ash" | "csh" | "tcsh"
    )
}

fn is_python_runtime(runtime: &str) -> bool {
    let lower = runtime.to_ascii_lowercase();
    lower == "python" || lower.starts_with("python3") || lower.starts_with("python2")
}

fn is_env_assignment_token(argument: &str) -> bool {
    let Some((lhs, _)) = argument.split_once('=') else {
        return false;
    };
    !lhs.is_empty()
        && !lhs.starts_with('-')
        && lhs
            .bytes()
            .enumerate()
            .all(|(i, b)| (i > 0 && b.is_ascii_digit()) || b.is_ascii_alphabetic() || b == b'_')
}

fn strip_leading_dot_slashes(path: &str) -> &str {
    let mut rest = path;
    while let Some(tail) = rest.strip_prefix("./").or_else(|| rest.strip_prefix(".\\")) {
        rest = tail;
    }
    rest
}

/// Whether an argument appearing before the `programs/...` file uses wrapper/inline/stdin execution.
fn is_pre_script_smuggling_arg(runtime: &str, argument: &str) -> bool {
    let lower = argument.to_ascii_lowercase();
    if argument == "-" || lower == "--stdin" || lower == "env" || is_env_assignment_token(argument)
    {
        return true;
    }
    if argument == "-s"
        || lower == "--rcfile"
        || lower.starts_with("--rcfile=")
        || lower == "--init-file"
        || lower.starts_with("--init-file=")
        || lower == "--command"
        || lower.starts_with("--command=")
    {
        return true;
    }
    if argument == "-m"
        || lower == "--module"
        || lower.starts_with("--module=")
        || (is_python_runtime(runtime) && argument.starts_with("-m"))
    {
        return true;
    }
    if is_shell_runtime(runtime)
        && argument.starts_with('-')
        && !argument.starts_with("--")
        && argument.len() > 1
        && argument[1..]
            .chars()
            .any(|ch| matches!(ch, 'c' | 's' | 'i' | 'm'))
    {
        return true;
    }
    false
}

/// Whether `argument` asks an interpreter to execute inline code rather than a package file.
fn is_inline_code_arg(argument: &str) -> bool {
    if INLINE_CODE.contains(&argument) {
        return true;
    }
    let lower = argument.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "-command" | "-encodedcommand" | "-enc" | "-ec" | "--input-type"
    ) || lower.starts_with("-command=")
        || lower.starts_with("-encodedcommand=")
        || lower.starts_with("-enc=")
        || lower.starts_with("-ec=")
        || argument.starts_with("--input-type=")
        || argument.starts_with("--eval=")
        || argument.starts_with("--print=")
}

fn windows_script(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.ends_with(".cmd") || lower.ends_with(".bat")
}

fn check_arguments(
    capsule: &SkillCapsule,
    files: &Files,
    arguments: &[String],
    at: &str,
    violations: &mut Vec<Violation>,
) {
    let Some(runtime) = arguments.first() else {
        violations.push(Violation::new(
            "argv_empty",
            at,
            "A program names the interpreter that runs it and its file.",
        ));
        return;
    };
    if is_disallowed_wrapper_runtime(runtime) {
        violations.push(Violation::new(
            "argv_inline_code",
            format!("{at}[0]"),
            format!("`{runtime}` is a process wrapper, not a direct language interpreter."),
        ));
    }
    if !capsule.capabilities.runtimes.contains(runtime) {
        violations.push(Violation::new(
            "runtime_undeclared",
            format!("{at}[0]"),
            format!("`{runtime}` is not one of `capabilities.runtimes`."),
        ));
    }
    let first_program_idx = arguments.iter().enumerate().skip(1).find_map(|(idx, arg)| {
        if arg.starts_with('-') || is_env_assignment_token(arg) {
            return None;
        }
        strip_leading_dot_slashes(arg)
            .starts_with("programs/")
            .then_some(idx)
    });
    let mut package_file = false;
    for (index, argument) in arguments.iter().enumerate() {
        let here = format!("{at}[{index}]");
        if windows_script(argument) {
            violations.push(Violation::new(
                "argv_windows_script",
                &here,
                "A program is never a `.cmd` or `.bat` launcher.",
            ));
        }
        if index == 0 {
            continue;
        }
        let before_script = first_program_idx.is_none_or(|pos| index < pos);
        if is_inline_code_arg(argument)
            || (before_script && is_pre_script_smuggling_arg(runtime, argument))
        {
            violations.push(Violation::new(
                "argv_inline_code",
                &here,
                "A program is a file of the package, not code or stdin/module/env redirection passed on the command line.",
            ));
            continue;
        }
        let path = argument
            .split_once('=')
            .map_or(argument.as_str(), |(_, value)| value);
        if path.starts_with('/')
            || path.starts_with('\\')
            || path.starts_with('~')
            || path.as_bytes().get(1) == Some(&b':')
        {
            violations.push(Violation::new(
                "argv_absolute_path",
                &here,
                "Arguments name package files by relative path only.",
            ));
            continue;
        }
        if path
            .split(['/', '\\'])
            .any(|part| part == ".." || (part.starts_with('-') && part.ends_with("..")))
        {
            violations.push(Violation::new(
                "argv_parent_traversal",
                &here,
                "An argument must not leave the package with `..`.",
            ));
            continue;
        }
        let normalized = {
            let mut rest = path;
            while let Some(tail) = rest.strip_prefix("./").or_else(|| rest.strip_prefix(".\\")) {
                rest = tail;
            }
            rest
        };
        if normalized.starts_with("programs/") || normalized.starts_with("assets/") {
            if !is_relative_path(path) || !files.contains(normalized) {
                let message = if path != normalized {
                    format!(
                        "`{path}` must omit leading `./` (use `{normalized}`) so the executor can resolve it."
                    )
                } else {
                    format!("`{path}` is not a file of the package.")
                };
                violations.push(Violation::new("argv_missing_file", &here, message));
            } else {
                package_file |= normalized.starts_with("programs/");
            }
        }
    }
    if !package_file {
        violations.push(Violation::new(
            "argv_no_program_file",
            at,
            "The command names no file under `programs/`.",
        ));
    }
}

fn define(
    defined: &mut BTreeMap<String, (&'static str, String, usize)>,
    name: &str,
    kind: &'static str,
    at: String,
    violations: &mut Vec<Violation>,
) {
    if defined.contains_key(name) {
        violations.push(Violation::new(
            "name_redefined",
            at,
            format!("`{name}` already names a binding, an output or a produced value."),
        ));
    } else {
        defined.insert(name.to_string(), (kind, at, 0));
    }
}

fn check_dataflow(capsule: &SkillCapsule, violations: &mut Vec<Violation>) {
    for (index, program) in capsule.programs.iter().enumerate() {
        if capsule.programs[..index]
            .iter()
            .any(|earlier| earlier.id == program.id)
        {
            violations.push(Violation::new(
                "duplicate_id",
                format!("programs[{index}].id"),
                format!("Program `{}` is defined twice.", program.id),
            ));
        }
    }
    // name -> (what defines it, where, how many later programs read it)
    let mut defined = BTreeMap::new();
    for (index, binding) in capsule.bindings.iter().enumerate() {
        define(
            &mut defined,
            &binding.name,
            "binding",
            format!("bindings[{index}].name"),
            violations,
        );
    }
    // program id -> (total uses, unparameterized uses)
    let mut uses: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for (index, step) in capsule.plan().enumerate() {
        let verification = capsule.is_verification(index);
        let at = if verification {
            format!("verification[{}]", index - capsule.steps.len())
        } else {
            format!("steps[{index}]")
        };
        match step {
            Step::Instruction { produces, .. } => {
                for (item, name) in produces.iter().enumerate() {
                    define(
                        &mut defined,
                        name,
                        "produces",
                        format!("{at}.produces[{item}]"),
                        violations,
                    );
                }
            }
            Step::Inspection { .. } => {}
            Step::Program {
                program: id,
                inputs: step_inputs,
                outputs: step_outputs,
            } => {
                let Some(program) = capsule.program(id) else {
                    violations.push(Violation::new(
                        "unknown_program",
                        format!("{at}.program"),
                        format!("No program is named `{id}`."),
                    ));
                    continue;
                };
                let parameterized = !step_inputs.is_empty() || !step_outputs.is_empty();
                let entry = uses.entry(id.as_str()).or_insert((0, 0));
                entry.0 += 1;
                if !parameterized {
                    entry.1 += 1;
                }
                if entry.0 > 1 && entry.1 > 1 {
                    violations.push(Violation::new(
                        "program_referenced_twice",
                        format!("{at}.program"),
                        format!("Program `{id}` already runs in an earlier step without parameterization."),
                    ));
                    continue;
                }
                if verification && !program.read_only {
                    violations.push(Violation::new(
                        "verification_not_read_only",
                        format!("{at}.program"),
                        format!("Verification program `{id}` must declare `read_only: true`."),
                    ));
                }
                let place = capsule
                    .programs
                    .iter()
                    .position(|candidate| candidate.id == *id)
                    .unwrap_or_default();
                for param in step_inputs.keys() {
                    if !program.inputs.contains(param) {
                        violations.push(Violation::new(
                            "unknown_input",
                            format!("{at}.inputs.{param}"),
                            format!("Program `{id}` declares no input named `{param}`."),
                        ));
                    }
                }
                for out in step_outputs.keys() {
                    if !program.outputs.contains(out) {
                        violations.push(Violation::new(
                            "unknown_output",
                            format!("{at}.outputs.{out}"),
                            format!("Program `{id}` declares no output named `{out}`."),
                        ));
                    }
                }
                let mut effective_inputs = Vec::with_capacity(program.inputs.len());
                for (item, input) in program.inputs.iter().enumerate() {
                    let effective = step_inputs
                        .get(input)
                        .map(String::as_str)
                        .unwrap_or(input.as_str());
                    effective_inputs.push(effective);
                    match defined.get_mut(effective) {
                        Some(entry) => entry.2 += 1,
                        None => violations.push(Violation::new(
                            "dangling_input",
                            if step_inputs.contains_key(input) {
                                format!("{at}.inputs.{input}")
                            } else {
                                format!("programs[{place}].inputs[{item}]")
                            },
                            format!(
                                "`{effective}` is not a binding, an earlier program's output or a value an earlier instruction produces."
                            ),
                        )),
                    }
                }
                for (item, output) in program.outputs.iter().enumerate() {
                    let effective = step_outputs
                        .get(output)
                        .map(String::as_str)
                        .unwrap_or(output.as_str());
                    let here = if step_outputs.contains_key(output) {
                        format!("{at}.outputs.{output}")
                    } else {
                        format!("programs[{place}].outputs[{item}]")
                    };
                    if program.inputs.contains(output) || effective_inputs.contains(&effective) {
                        violations.push(Violation::new(
                            "output_overlaps_input",
                            &here,
                            format!("`{effective}` is an input of the same program step."),
                        ));
                        continue;
                    }
                    define(&mut defined, effective, "output", here, violations);
                }
            }
        }
    }
    for (index, program) in capsule.programs.iter().enumerate() {
        if !uses.contains_key(program.id.as_str()) {
            violations.push(Violation::new(
                "program_unreferenced",
                format!("programs[{index}]"),
                format!("No step runs program `{}`.", program.id),
            ));
        }
    }
    for (name, (kind, at, readers)) in &defined {
        if *readers > 0 {
            continue;
        }
        let (code, message) = match *kind {
            "binding" => (
                "binding_unused",
                format!("No program reads binding `{name}`."),
            ),
            "produces" => (
                "produces_unread",
                format!("No later program reads `{name}`."),
            ),
            _ => (
                "output_unread",
                format!(
                    "No later program reads output `{name}`; an answer for the agent belongs in the result."
                ),
            ),
        };
        violations.push(Violation::new(code, at.clone(), message));
    }
}

fn check_sources(capsule: &SkillCapsule, violations: &mut Vec<Violation>) {
    let folders = capsule
        .skills
        .iter()
        .map(|skill| {
            skill
                .path
                .rsplit_once('/')
                .map_or(String::new(), |(folder, _)| format!("{folder}/"))
        })
        .collect::<Vec<_>>();
    for (index, step) in capsule.plan().enumerate() {
        let (Step::Instruction {
            source: Some(source),
            ..
        }
        | Step::Inspection {
            source: Some(source),
            ..
        }) = step
        else {
            continue;
        };
        let inside = capsule.skills.iter().any(|skill| skill.path == source.path)
            || folders
                .iter()
                .any(|folder| !folder.is_empty() && source.path.starts_with(folder.as_str()));
        if !inside {
            let at = if capsule.is_verification(index) {
                format!("verification[{}].source.path", index - capsule.steps.len())
            } else {
                format!("steps[{index}].source.path")
            };
            violations.push(Violation::new(
                "source_outside_skill",
                at,
                "A step points into one of the capsule's source skills.",
            ));
        }
    }
}

fn normalized(text: &str) -> String {
    text.replace("\r\n", "\n").trim().to_string()
}

fn is_compound_or_path_literal(text: &str) -> bool {
    text.contains(['/', '\\', ':', '\n'])
        || text.contains(char::is_whitespace)
        || (text.contains('.') && text.chars().any(char::is_alphanumeric))
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn contains_literal(haystack: &str, literal: &str) -> bool {
    if literal.is_empty() {
        return false;
    }
    if is_compound_or_path_literal(literal) {
        return haystack.contains(literal);
    }
    let mut start = 0;
    while let Some(found) = haystack[start..].find(literal) {
        let idx = start + found;
        let end = idx + literal.len();
        let left_ok = haystack[..idx]
            .chars()
            .next_back()
            .is_none_or(|c| !is_word_char(c));
        let right_ok = haystack[end..]
            .chars()
            .next()
            .is_none_or(|c| !is_word_char(c));
        if left_ok && right_ok {
            return true;
        }
        start = idx + haystack[idx..].chars().next().map_or(1, char::len_utf8);
    }
    false
}

/// Collects text strings from `capsule` that carry author-written descriptions
/// or commands, excluding JSON structure keys.
fn capsule_authored_strings(capsule: &SkillCapsule) -> Vec<&str> {
    let mut strings = vec![capsule.mu.name.as_str(), capsule.mu.description.as_str()];
    for binding in &capsule.bindings {
        if let Some(desc) = &binding.description {
            strings.push(desc.as_str());
        }
    }
    for program in &capsule.programs {
        if let Some(desc) = &program.description {
            strings.push(desc.as_str());
        }
        strings.extend(program.entry.iter().map(String::as_str));
        strings.extend(program.check.iter().map(String::as_str));
    }
    for step in capsule.steps.iter().chain(capsule.verification.iter()) {
        match step {
            Step::Instruction { text, .. } | Step::Inspection { text, .. } => {
                strings.push(text.as_str())
            }
            Step::Program { .. } => {}
        }
    }
    strings
}

/// Extracts quoted string literals (`'...'`, `"..."`, `` `...` ``, triple quotes)
/// from a package source file, ignoring comment-only prose.
fn extract_source_literals(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'#' || (b == b'/' && bytes.get(i + 1) == Some(&b'/')) {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if matches!(b, b'\'' | b'"' | b'`') {
            let triple = b != b'`' && bytes.get(i + 1) == Some(&b) && bytes.get(i + 2) == Some(&b);
            let quote_len = if triple { 3 } else { 1 };
            i += quote_len;
            let mut literal = String::new();
            while i < bytes.len() {
                if triple {
                    if bytes.get(i..i + 3) == Some(&[b, b, b]) {
                        i += 3;
                        break;
                    }
                } else if bytes[i] == b {
                    i += 1;
                    break;
                }
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    let esc = bytes[i + 1];
                    match esc {
                        b'n' => literal.push('\n'),
                        b'r' => literal.push('\r'),
                        b't' => literal.push('\t'),
                        _ => literal.push(esc as char),
                    }
                    i += 2;
                    continue;
                }
                let ch = source[i..].chars().next().unwrap_or('\0');
                literal.push(ch);
                i += ch.len_utf8();
            }
            out.push(literal);
            continue;
        }
        i += source[i..].chars().next().map_or(1, char::len_utf8);
    }
    out
}

/// Non-comment lines of `source` for checking unquoted path-like literals.
fn source_non_comment_code(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') || trimmed.starts_with("//") || trimmed.starts_with("--") {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

const GENERIC_SHORT_LITERALS: &[&str] = &[
    "csv", "tsv", "pdf", "json", "xml", "yaml", "yml", "toml", "html", "text", "txt", "md", "utf8",
    "utf-8", "ascii", "true", "false", "null", "none", "ok", "done", "pass", "fail", "read",
    "write", "check", "list", "all", "main", "test", "file", "files", "path", "name", "data",
    "item", "items", "count", "total", "input", "output", "error", "status", "complete", "result",
];

fn is_generic_short_literal(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    GENERIC_SHORT_LITERALS.contains(&lower.as_str())
}

fn contains_number_token(haystack: &str, number: &str) -> bool {
    if number.is_empty() {
        return false;
    }
    let mut start = 0;
    while let Some(found) = haystack[start..].find(number) {
        let idx = start + found;
        let end = idx + number.len();
        let left = haystack[..idx].chars().next_back();
        let right = haystack[end..].chars().next();
        let left_ok = left.is_none_or(|c| {
            !c.is_alphanumeric()
                && c != '_'
                && c != '.'
                && c != '+'
                && (c != '-' || number.starts_with('-'))
        });
        let right_ok = right.is_none_or(|c| !c.is_alphanumeric() && c != '_' && c != '.');
        // Ignore Markdown/prose ordered list prefixes like `3. Open ...` at line starts.
        let ordered_list_prefix = left.is_none_or(|c| matches!(c, '\n' | '\r' | ' ' | '\t'))
            && haystack[end..].starts_with(". ");
        let preceded_only_by_ws = haystack[..idx]
            .rsplit_once('\n')
            .map_or(&haystack[..idx], |(_, line)| line)
            .trim()
            .is_empty();
        if left_ok && right_ok && !(ordered_list_prefix && preceded_only_by_ws) {
            return true;
        }
        start = idx + haystack[idx..].chars().next().map_or(1, char::len_utf8);
    }
    false
}

fn non_trivial_example_scalar(text: &str, is_number: bool) -> bool {
    if text.is_empty() {
        return false;
    }
    if is_number {
        if matches!(
            text,
            "-1" | "0" | "1" | "2" | "0.0" | "1.0" | "-1.0" | "2.0"
        ) {
            return false;
        }
        return true;
    }
    let count = text.chars().count();
    if count >= EXAMPLE_LITERAL_CHARS {
        return true;
    }
    if count >= 3 {
        return !is_generic_short_literal(text);
    }
    if count == 2 {
        return text
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            || text.contains(['/', '\\']);
    }
    false
}

fn collect_example_scalars(value: &Value, out: &mut Vec<(String, bool)>) {
    match value {
        Value::String(text) => out.push((normalized(text), false)),
        Value::Number(num) => out.push((num.to_string(), true)),
        Value::Array(items) => {
            for item in items {
                collect_example_scalars(item, out);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                collect_example_scalars(item, out);
            }
        }
        _ => {}
    }
}

fn literal_matches_text(haystack: &str, literal: &str, is_number: bool) -> bool {
    if is_number {
        contains_number_token(haystack, literal)
    } else {
        contains_literal(haystack, literal)
    }
}

fn source_contains_example_scalar(source: &str, text: &str, is_number: bool) -> bool {
    if is_number {
        return contains_number_token(&source_non_comment_code(source), text);
    }
    if extract_source_literals(source)
        .iter()
        .any(|lit| contains_literal(lit, text))
    {
        return true;
    }
    is_compound_or_path_literal(text) && contains_literal(&source_non_comment_code(source), text)
}

#[cfg(test)]
fn source_contains_example_literal(source: &str, text: &str) -> bool {
    source_contains_example_scalar(source, text, false)
}

fn check_smuggled_example_value(
    capsule: &SkillCapsule,
    package_files: &Files,
    at: &str,
    value: &Value,
    violations: &mut Vec<Violation>,
) -> bool {
    let mut scalars = Vec::new();
    collect_example_scalars(value, &mut scalars);
    for (text, is_number) in scalars {
        if !non_trivial_example_scalar(&text, is_number) {
            continue;
        }
        if capsule_authored_strings(capsule)
            .iter()
            .any(|field| literal_matches_text(field, &text, is_number))
        {
            violations.push(Violation::new(
                "example_value_in_capsule",
                at,
                "The capsule repeats this observed value; a capsule carries the parameter, not the instance.",
            ));
            return true;
        }
        if let Some((path, _)) = package_files.iter().find(|(_, content)| {
            std::str::from_utf8(content)
                .is_ok_and(|source| source_contains_example_scalar(source, &text, is_number))
        }) {
            violations.push(Violation::new(
                "example_value_in_capsule",
                at,
                format!(
                    "The package file `{path}` repeats this observed value; a capsule carries the parameter, not the instance."
                ),
            ));
            return true;
        }
    }
    false
}

fn check_examples(
    capsule: &SkillCapsule,
    package_files: &Files,
    _capsule_text: &str,
    examples: &Examples,
    context: &Context<'_>,
    violations: &mut Vec<Violation>,
) {
    let produced = capsule
        .steps
        .iter()
        .filter_map(|step| match step {
            Step::Instruction { produces, .. } => Some(produces.iter()),
            _ => None,
        })
        .flatten()
        .collect::<Vec<_>>();
    for binding in &capsule.bindings {
        if !examples.bindings.contains_key(&binding.name) {
            violations.push(Violation::new(
                "example_missing",
                format!("examples.bindings.{}", binding.name),
                format!("No observed value for binding `{}`.", binding.name),
            ));
        }
    }
    for name in &produced {
        if !examples.bindings.contains_key(name.as_str()) {
            violations.push(Violation::new(
                "example_missing",
                format!("examples.bindings.{name}"),
                format!("No observed value for `{name}`, which an instruction produces."),
            ));
        }
    }
    let files = context
        .observed_files
        .iter()
        .map(|content| normalized(content))
        .collect::<Vec<_>>();
    for (name, value) in &examples.bindings {
        let at = format!("examples.bindings.{name}");
        let declared = capsule.binding(name);
        if declared.is_none() && !produced.contains(&name) {
            violations.push(Violation::new(
                "example_unknown_name",
                &at,
                format!("`{name}` is neither a binding nor a produced value."),
            ));
            continue;
        }
        if let Some(binding) = declared
            && !binding.kind.admits(value)
        {
            violations.push(Violation::new(
                "binding_type_mismatch",
                &at,
                format!(
                    "The observed value is not a `{}` as declared.",
                    binding.kind.as_str()
                ),
            ));
        } else if declared.is_none() && !is_admitted_value(value) {
            violations.push(Violation::new(
                "invalid_type",
                &at,
                "An observed value is a scalar or a list of scalars.",
            ));
        }
        let size = serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len());
        if size > BINDING_BYTES {
            violations.push(Violation::new(
                "binding_too_large",
                &at,
                format!("A bound value is at most {BINDING_BYTES} bytes."),
            ));
            continue;
        }
        let texts: Vec<&str> = match value {
            Value::String(text) => vec![text.as_str()],
            Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        for text in texts {
            let text = normalized(text);
            if text.is_empty() {
                continue;
            }
            let whole_file = files.iter().any(|file| !file.is_empty() && *file == text);
            let part_of_file =
                text.lines().count() >= 3 && files.iter().any(|file| file.contains(&text));
            if whole_file || part_of_file {
                violations.push(Violation::new(
                    "file_content_binding",
                    &at,
                    "The value is the content of a file the observed run read; bind the path instead.",
                ));
                break;
            }
        }
        check_smuggled_example_value(capsule, package_files, &at, value, violations);
    }
    for (index, result) in &examples.steps {
        let at = format!("examples.steps.{index}");
        let Some(program) = capsule
            .plan_step(*index)
            .and_then(Step::program)
            .and_then(|id| capsule.program(id))
        else {
            violations.push(Violation::new(
                "example_unknown_step",
                &at,
                "An observed result belongs to a program step of the plan.",
            ));
            continue;
        };
        for output in &program.outputs {
            let here = format!("{at}.result.{output}");
            match result.get(output) {
                None | Some(Value::Null) => violations.push(Violation::new(
                    "example_missing_output",
                    here,
                    format!("The observed result lacks declared output `{output}`."),
                )),
                Some(value) if !is_admitted_value(value) => violations.push(Violation::new(
                    "invalid_type",
                    here,
                    "An observed output is a scalar or a list of scalars.",
                )),
                Some(value)
                    if serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
                        > BINDING_BYTES =>
                {
                    violations.push(Violation::new(
                        "binding_too_large",
                        here,
                        format!("An observed output is at most {BINDING_BYTES} bytes."),
                    ));
                }
                Some(value) => {
                    check_smuggled_example_value(capsule, package_files, &here, value, violations);
                }
            }
        }
        for (field, value) in result {
            if program.outputs.contains(field) {
                continue;
            }
            let here = format!("{at}.{field}");
            check_smuggled_example_value(capsule, package_files, &here, value, violations);
        }
        if serde_json::to_vec(result).map_or(usize::MAX, |bytes| bytes.len()) > 64 * 1024 {
            violations.push(Violation::new(
                "size_limit",
                &at,
                "An observed result is at most 64 KiB.",
            ));
        }
    }
    for (index, step) in capsule.plan().enumerate() {
        if step
            .program()
            .and_then(|id| capsule.program(id))
            .is_some_and(|program| !program.outputs.is_empty())
            && !examples.steps.contains_key(&index)
        {
            violations.push(Violation::new(
                "example_missing",
                format!("examples.steps.{index}"),
                "No observed result for a program whose outputs later programs read.",
            ));
        }
    }
}

fn check_secrets(label: &str, text: &str, violations: &mut Vec<Violation>) {
    match infra_core::domain::secret_label(text) {
        Ok(None) => {}
        Ok(Some(kind)) => violations.push(Violation::new(
            "secret_detected",
            label,
            format!("The text carries a possible {kind}; the value is not repeated here."),
        )),
        Err(_) => violations.push(Violation::new(
            "secret_policy_unavailable",
            label,
            "The secret policy could not be applied, so the text is refused.",
        )),
    }
}

fn check_bindings(capsule: &SkillCapsule, violations: &mut Vec<Violation>) {
    for (index, binding) in capsule.bindings.iter().enumerate() {
        let at = format!("bindings[{index}]");
        match binding.origin {
            BindingOrigin::Request => {
                if binding.pattern.is_some() {
                    violations.push(Violation::new(
                        "binding_pattern_forbidden",
                        format!("{at}.pattern"),
                        "`pattern` is only valid for `workspace_path`, `route_arg`, or `prompt_regex` bindings.",
                    ));
                }
            }
            BindingOrigin::PromptRegex => match &binding.pattern {
                Some(pattern) => {
                    if let Err(err) = compile_bounded_binding_regex(pattern) {
                        violations.push(Violation::new(
                            "binding_pattern_invalid",
                            format!("{at}.pattern"),
                            err,
                        ));
                    }
                }
                None => violations.push(Violation::new(
                    "binding_pattern_missing",
                    format!("{at}.pattern"),
                    "A `prompt_regex` binding requires a non-empty bounded `pattern`.",
                )),
            },
            BindingOrigin::RouteArg => {
                if let Some(pattern) = &binding.pattern
                    && let Err(err) = compile_bounded_binding_regex(pattern)
                {
                    violations.push(Violation::new(
                        "binding_pattern_invalid",
                        format!("{at}.pattern"),
                        err,
                    ));
                }
            }
            BindingOrigin::WorkspacePath => {
                if !matches!(binding.kind, BindingType::Path | BindingType::String) {
                    violations.push(Violation::new(
                        "binding_type_mismatch",
                        format!("{at}.type"),
                        "A `workspace_path` binding must have type `path` or `string`.",
                    ));
                }
                if let Some(pattern) = &binding.pattern
                    && let Err(err) = compile_bounded_binding_regex(pattern)
                {
                    violations.push(Violation::new(
                        "binding_pattern_invalid",
                        format!("{at}.pattern"),
                        err,
                    ));
                }
            }
        }
        if let Some(default) = &binding.default {
            if !value_within_binding_limit(binding.kind, default) {
                violations.push(Violation::new(
                    "binding_default_invalid",
                    format!("{at}.default"),
                    format!(
                        "`default` must satisfy binding type `{}` within {BINDING_BYTES} bytes.",
                        binding.kind.as_str()
                    ),
                ));
            }
            check_secrets(&format!("{at}.default"), &default.to_string(), violations);
        }
    }
}

/// The rules of a hybrid capsule that need only the capsule and its package:
/// the plan's dataflow, the steps' sources and the programs' commands. An
/// executor applies them again before it runs anything of a stored package.
pub fn structure(capsule: &SkillCapsule, files: &Files) -> Vec<Violation> {
    let mut violations = Vec::new();
    if capsule.kind == CapsuleKind::ToolChain {
        if !capsule.skills.is_empty() {
            violations.push(Violation::new(
                "tool_chain_has_skill",
                "source.skills",
                "A tool-chain capsule has no source skill.",
            ));
        }
        for (index, program) in capsule.programs.iter().enumerate() {
            if !program.read_only {
                violations.push(Violation::new(
                    "tool_chain_not_read_only",
                    format!("programs[{index}].read_only"),
                    "A tool-chain capsule only runs read-only or verification programs.",
                ));
            }
        }
    }
    if capsule.programs.is_empty() {
        violations.push(Violation::new(
            "hybrid_without_programs",
            "programs",
            "A hybrid capsule runs at least one program; a procedure without programs is the textual form.",
        ));
    }
    if capsule.steps.is_empty() {
        violations.push(Violation::new(
            "empty_plan",
            "steps",
            "A capsule has at least one step.",
        ));
    }
    check_bindings(capsule, &mut violations);
    check_dataflow(capsule, &mut violations);
    check_sources(capsule, &mut violations);
    for (index, program) in capsule.programs.iter().enumerate() {
        check_arguments(
            capsule,
            files,
            &program.entry,
            &format!("programs[{index}].entry"),
            &mut violations,
        );
        check_arguments(
            capsule,
            files,
            &program.check,
            &format!("programs[{index}].check"),
            &mut violations,
        );
    }
    violations
}

/// Gate 1 for a hybrid candidate. `capsule` is `capsule.json`, `files` the
/// package beside it and `examples` the observed values, when there are any.
pub fn check(
    capsule: &[u8],
    files: &Files,
    examples: Option<&[u8]>,
    context: &Context<'_>,
) -> Report {
    let mut violations = Vec::new();
    let parsed = match SkillCapsule::parse(capsule) {
        Ok(parsed) => Some(parsed),
        Err(found) => {
            violations.extend(found);
            None
        }
    };
    let examples = examples.and_then(|bytes| match Examples::parse(bytes) {
        Ok(examples) => {
            check_secrets("examples", &String::from_utf8_lossy(bytes), &mut violations);
            Some(examples)
        }
        Err(found) => {
            violations.extend(found);
            None
        }
    });
    violations.extend(files.violations());
    let text = String::from_utf8_lossy(capsule);
    check_secrets("capsule", &text, &mut violations);
    for (path, content) in files.iter() {
        if let Ok(content) = std::str::from_utf8(content) {
            check_secrets(path, content, &mut violations);
        }
    }
    let Some(parsed) = parsed else {
        return Report::of("gate1", violations);
    };
    if parsed.form != Form::Hybrid {
        violations.push(Violation::new(
            "invalid_value",
            "form",
            "Gate 1 checks the hybrid form; the textual form has its own gate.",
        ));
        return Report::of("gate1", violations);
    }
    violations.extend(structure(&parsed, files));
    if let Some(examples) = &examples {
        check_examples(&parsed, files, &text, examples, context, &mut violations);
    }
    Report::of("gate1", violations)
}

/// Gate 1T for the textual form: the manifest, the Markdown's shape and size,
/// and the absence of secrets and observed values. Nothing is executed for a
/// textual capsule, so Gates 2 and 3 do not apply to it.
pub fn check_textual(capsule: &[u8], markdown: &[u8], examples: Option<&Examples>) -> Report {
    let mut violations = Vec::new();
    match SkillCapsule::parse(capsule) {
        Ok(parsed) if parsed.form != Form::Textual => violations.push(Violation::new(
            "invalid_value",
            "form",
            "Gate 1T checks the textual form.",
        )),
        Ok(_) => {}
        Err(found) => violations.extend(found),
    }
    check_secrets(
        "capsule",
        &String::from_utf8_lossy(capsule),
        &mut violations,
    );
    if markdown.len() > MARKDOWN_BYTES {
        violations.push(Violation::new(
            "size_limit",
            "capsule.md",
            format!("The written procedure is at most {MARKDOWN_BYTES} bytes."),
        ));
    }
    let Ok(text) = std::str::from_utf8(markdown) else {
        violations.push(Violation::new(
            "invalid_markdown",
            "capsule.md",
            "The written procedure is UTF-8 text.",
        ));
        return Report::of("gate1t", violations);
    };
    let lines = text.lines().map(str::trim_end).collect::<Vec<_>>();
    let first = lines.iter().find(|line| !line.trim().is_empty());
    if !first.is_some_and(|line| line.starts_with("# ") && line.len() > 2) {
        violations.push(Violation::new(
            "invalid_markdown",
            "capsule.md",
            "The procedure starts with one `# ` title.",
        ));
    }
    let numbered = |line: &&str| {
        let line = line.trim_start();
        let digits = line.bytes().take_while(u8::is_ascii_digit).count();
        digits > 0 && line[digits..].starts_with(". ")
    };
    if !lines.iter().any(numbered) {
        violations.push(Violation::new(
            "invalid_markdown",
            "capsule.md",
            "The procedure lists its steps as a numbered list.",
        ));
    }
    if lines.iter().filter(|line| line.starts_with("```")).count() % 2 != 0 {
        violations.push(Violation::new(
            "invalid_markdown",
            "capsule.md",
            "A fenced block is not closed.",
        ));
    }
    check_secrets("capsule.md", text, &mut violations);
    if let Some(examples) = examples {
        for (name, value) in &examples.bindings {
            let mut scalars = Vec::new();
            collect_example_scalars(value, &mut scalars);
            for (raw, is_number) in &scalars {
                if !*is_number {
                    check_secrets(&format!("examples.bindings.{name}"), raw, &mut violations);
                }
            }
            if scalars.iter().any(|(literal, is_number)| {
                non_trivial_example_scalar(literal, *is_number)
                    && literal_matches_text(text, literal, *is_number)
            }) {
                violations.push(Violation::new(
                    "example_value_in_capsule",
                    format!("examples.bindings.{name}"),
                    "The procedure repeats this observed value; it describes the parameter, not the instance.",
                ));
            }
        }
    }
    Report::of("gate1t", violations)
}

fn find_non_portable_path(text: &str) -> Option<&'static str> {
    for marker in [
        "/Users/",
        "/home/",
        "file:///Users/",
        "file:///home/",
        "\\\\Users\\\\",
        "/.worktrees/",
    ] {
        if text.contains(marker) {
            return Some(marker);
        }
    }
    let bytes = text.as_bytes();
    for (idx, window) in bytes.windows(2).enumerate() {
        let prev = idx.checked_sub(1).and_then(|i| bytes.get(i).copied());
        let boundary =
            prev.is_none_or(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'"' | b'\'' | b'='));
        if boundary && window == b"~/" {
            return Some("~/");
        }
    }
    for (idx, window) in bytes.windows(3).enumerate() {
        let prev = idx.checked_sub(1).and_then(|i| bytes.get(i).copied());
        let boundary =
            prev.is_none_or(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'"' | b'\'' | b'='));
        if boundary
            && window[0].is_ascii_alphabetic()
            && window[1] == b':'
            && matches!(window[2], b'\\' | b'/')
        {
            return Some("drive-path");
        }
    }
    None
}

fn check_portable_text(label: &str, text: &str, violations: &mut Vec<Violation>) {
    check_secrets(label, text, violations);
    if let Some(marker) = find_non_portable_path(text) {
        violations.push(Violation::new(
            "template_non_portable_path",
            label,
            format!(
                "Edition template capsules must not embed host-specific or absolute paths ({marker})."
            ),
        ));
    }
}

/// Gate 1 portability check for edition templates before storage or target admission.
pub fn check_portable_template(capsule: &[u8], files: &Files, markdown: Option<&[u8]>) -> Report {
    let mut violations = Vec::new();
    let parsed = match SkillCapsule::parse(capsule) {
        Ok(parsed) => Some(parsed),
        Err(found) => {
            violations.extend(found);
            None
        }
    };
    violations.extend(files.violations());
    let text = String::from_utf8_lossy(capsule);
    check_portable_text("capsule", &text, &mut violations);
    for (path, content) in files.iter() {
        if let Ok(content) = std::str::from_utf8(content) {
            check_portable_text(path, content, &mut violations);
        }
    }
    if let Some(markdown) = markdown
        && let Ok(md_text) = std::str::from_utf8(markdown)
    {
        check_portable_text("capsule.md", md_text, &mut violations);
    }
    if let Some(parsed) = parsed {
        if parsed.skills.is_empty() {
            violations.push(Violation::new(
                "missing_field",
                "source.skills",
                "An edition template names the edition skill it came from.",
            ));
        }
        if parsed.form == Form::Hybrid {
            violations.extend(structure(&parsed, files));
        }
    }
    Report::of("gate1_template", violations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::Files;
    use serde_json::json;

    #[test]
    fn contains_literal_ignores_comments_and_partial_identifier_substrings() {
        assert!(!contains_literal("documentation helper", "document"));
        assert!(!contains_literal(
            "build_invoice_data_cache",
            "invoice_data"
        ));
        assert!(contains_literal("read \"document\" here", "document"));
        assert!(contains_literal("invoices/2026-q3/", "invoices/2026-q3/"));

        let py = b"# example comment: invoices/2026-q3/ and document\ndef build_document_id():\n    return 1\n";
        let src = std::str::from_utf8(py).unwrap();
        assert!(!source_contains_example_literal(src, "invoices/2026-q3/"));
        assert!(!source_contains_example_literal(src, "document"));
        assert!(source_contains_example_literal(
            "print('invoices/2026-q3/')\n",
            "invoices/2026-q3/"
        ));
    }

    #[test]
    fn tool_chain_capsule_requires_read_only_programs() {
        let mut val = json!({
            "schema_version": 1,
            "form": "hybrid",
            "kind": "tool_chain",
            "mu": {"name": "repo-verify", "description": "Read-only repo inspection"},
            "source": {"skills": [], "tool_chain": {"tree_sha256": "b".repeat(64)}},
            "capabilities": {"network": "none", "runtimes": ["python3"]},
            "bindings": [],
            "programs": [{
                "id": "inspect",
                "entry": ["python3", "programs/inspect/main.py"],
                "inputs": [],
                "outputs": [],
                "check": ["python3", "programs/inspect/main.py", "--check"],
                "timeout_seconds": 30,
                "read_only": false
            }],
            "steps": [{"kind": "program", "program": "inspect"}],
            "verification": []
        });
        let files = Files::from_response(&json!([
            {"path": "programs/inspect/main.py", "content": "print('ok')\n"}
        ]));
        let parsed = SkillCapsule::from_value(&val).unwrap();
        let bad = structure(&parsed, &files);
        assert!(bad.iter().any(|v| v.code == "tool_chain_not_read_only"));

        val["programs"][0]["read_only"] = json!(true);
        let parsed_ok = SkillCapsule::from_value(&val).unwrap();
        let good = structure(&parsed_ok, &files);
        assert!(good.is_empty(), "{good:?}");
    }

    #[test]
    fn portable_template_rejects_host_paths_and_secrets() {
        let cap = json!({
            "schema_version": 1,
            "form": "hybrid",
            "kind": "template",
            "mu": {"name": "laravel-check", "description": "Check Laravel migrations"},
            "source": {
                "edition": "Laravel",
                "skills": [{"name": "laravel-check", "path": ".claude/skills/laravel-check/SKILL.md", "tree_sha256": "c".repeat(64)}]
            },
            "capabilities": {"network": "none", "runtimes": ["python3"]},
            "bindings": [],
            "programs": [{
                "id": "check",
                "entry": ["python3", "programs/check/main.py"],
                "inputs": [],
                "outputs": [],
                "check": ["python3", "programs/check/main.py", "--check"],
                "timeout_seconds": 30,
                "read_only": true
            }],
            "steps": [{"kind": "program", "program": "check"}],
            "verification": []
        });
        let bytes = serde_json::to_vec(&cap).unwrap();
        let bad_files = Files::from_response(&json!([
            {"path": "programs/check/main.py", "content": "PATH = '/home/alice/secret/app'\n"}
        ]));
        let bad = check_portable_template(&bytes, &bad_files, None);
        assert!(!bad.passed);
        assert!(bad.has("template_non_portable_path"));

        let good_files = Files::from_response(&json!([
            {"path": "programs/check/main.py", "content": "#!/usr/bin/env python3\nprint('portable')\n"}
        ]));
        let good = check_portable_template(&bytes, &good_files, None);
        assert!(good.passed, "{:?}", good.violations);
    }

    #[test]
    fn binding_origins_patterns_and_defaults_validate() {
        let base = |bindings: Value| -> Value {
            json!({
                "schema_version": 1,
                "form": "hybrid",
                "mu": {"name": "param-ro", "description": "Parameterized read-only"},
                "source": {"skills": [{"name": "s", "path": ".claude/skills/s/SKILL.md", "tree_sha256": "d".repeat(64)}]},
                "capabilities": {"network": "none", "runtimes": ["python3"]},
                "bindings": bindings,
                "programs": [{
                    "id": "scan",
                    "entry": ["python3", "programs/scan/main.py"],
                    "inputs": ["target"],
                    "outputs": [],
                    "check": ["python3", "programs/scan/main.py", "--check"],
                    "timeout_seconds": 30,
                    "read_only": true
                }],
                "steps": [{"kind": "program", "program": "scan"}],
                "verification": []
            })
        };
        let files = Files::from_response(&json!([
            {"path": "programs/scan/main.py", "content": "print('ok')\n"}
        ]));

        // Good prompt_regex + default passes.
        let ok = SkillCapsule::from_value(&base(json!([{
            "name": "target",
            "type": "string",
            "origin": "prompt_regex",
            "pattern": r"\b(inv-\d+)\b",
            "default": "inv-0"
        }])))
        .unwrap();
        assert!(structure(&ok, &files).is_empty());

        // Missing pattern on prompt_regex fails.
        let missing = SkillCapsule::from_value(&base(json!([{
            "name": "target",
            "type": "string",
            "origin": "prompt_regex"
        }])))
        .unwrap();
        assert!(
            structure(&missing, &files)
                .iter()
                .any(|v| v.code == "binding_pattern_missing")
        );

        // Request origin with pattern forbidden.
        let req_pat = SkillCapsule::from_value(&base(json!([{
            "name": "target",
            "type": "string",
            "origin": "request",
            "pattern": "abc"
        }])))
        .unwrap();
        assert!(
            structure(&req_pat, &files)
                .iter()
                .any(|v| v.code == "binding_pattern_forbidden")
        );

        // Empty-matching pattern rejected.
        let empty_re = SkillCapsule::from_value(&base(json!([{
            "name": "target",
            "type": "string",
            "origin": "prompt_regex",
            "pattern": ".*"
        }])))
        .unwrap();
        assert!(
            structure(&empty_re, &files)
                .iter()
                .any(|v| v.code == "binding_pattern_invalid")
        );

        // workspace_path requires path/string type, and path default rejects traversal.
        let bad_ws_type = SkillCapsule::from_value(&base(json!([{
            "name": "target",
            "type": "number",
            "origin": "workspace_path"
        }])))
        .unwrap();
        assert!(
            structure(&bad_ws_type, &files)
                .iter()
                .any(|v| v.code == "binding_type_mismatch")
        );
        let bad_def = SkillCapsule::from_value(&base(json!([{
            "name": "target",
            "type": "path",
            "origin": "workspace_path",
            "default": "../etc/passwd"
        }])))
        .unwrap();
        assert!(
            structure(&bad_def, &files)
                .iter()
                .any(|v| v.code == "binding_default_invalid")
        );
    }
}
