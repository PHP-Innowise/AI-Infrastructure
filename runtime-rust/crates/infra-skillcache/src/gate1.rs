//! Gate 1: a capsule's structure and dataflow, checked without running anything.
//!
//! The gate reads a candidate as the compiler returned it: the capsule, the
//! package files and the observed values. It reports every violation it can
//! see in one pass, each with a stable code, so one repair round can answer
//! all of them. Gate 1T is the lighter check of the textual form.
use crate::capsule::{
    BINDING_BYTES, Examples, Form, SkillCapsule, Step, Violation, is_admitted_value,
    is_relative_path,
};
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
    "-c", "-e", "-E", "--eval", "-p", "--print", "-Command", "/c", "/C", "/k", "/K",
];

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
    if !capsule.capabilities.runtimes.contains(runtime) {
        violations.push(Violation::new(
            "runtime_undeclared",
            format!("{at}[0]"),
            format!("`{runtime}` is not one of `capabilities.runtimes`."),
        ));
    }
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
        if INLINE_CODE.contains(&argument.as_str()) {
            violations.push(Violation::new(
                "argv_inline_code",
                &here,
                "A program is a file of the package, not code passed on the command line.",
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
                violations.push(Violation::new(
                    "argv_missing_file",
                    &here,
                    format!("`{path}` is not a file of the package."),
                ));
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
    let mut uses: BTreeMap<&str, usize> = BTreeMap::new();
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
            Step::Program { program: id } => {
                let Some(program) = capsule.program(id) else {
                    violations.push(Violation::new(
                        "unknown_program",
                        format!("{at}.program"),
                        format!("No program is named `{id}`."),
                    ));
                    continue;
                };
                *uses.entry(id.as_str()).or_default() += 1;
                if uses[id.as_str()] > 1 {
                    violations.push(Violation::new(
                        "program_referenced_twice",
                        format!("{at}.program"),
                        format!("Program `{id}` already runs in an earlier step."),
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
                for (item, input) in program.inputs.iter().enumerate() {
                    match defined.get_mut(input) {
                        Some(entry) => entry.2 += 1,
                        None => violations.push(Violation::new(
                            "dangling_input",
                            format!("programs[{place}].inputs[{item}]"),
                            format!(
                                "`{input}` is not a binding, an earlier program's output or a value an earlier instruction produces."
                            ),
                        )),
                    }
                }
                for (item, output) in program.outputs.iter().enumerate() {
                    let here = format!("programs[{place}].outputs[{item}]");
                    if program.inputs.contains(output) {
                        violations.push(Violation::new(
                            "output_overlaps_input",
                            &here,
                            format!("`{output}` is an input of the same program."),
                        ));
                        continue;
                    }
                    define(&mut defined, output, "output", here, violations);
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

fn contains_literal(capsule_text: &str, literal: &str) -> bool {
    if capsule_text.contains(literal) {
        return true;
    }
    serde_json::to_string(literal)
        .ok()
        .and_then(|quoted| {
            quoted
                .len()
                .checked_sub(2)
                .map(|_| quoted[1..quoted.len() - 1].to_string())
        })
        .is_some_and(|escaped| capsule_text.contains(&escaped))
}

fn check_examples(
    capsule: &SkillCapsule,
    capsule_text: &str,
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
            if text.chars().count() >= EXAMPLE_LITERAL_CHARS
                && contains_literal(capsule_text, &text)
            {
                violations.push(Violation::new(
                    "example_value_in_capsule",
                    &at,
                    "The capsule repeats this observed value; a capsule carries the parameter, not the instance.",
                ));
                break;
            }
        }
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
                Some(_) => {}
            }
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

/// The rules of a hybrid capsule that need only the capsule and its package:
/// the plan's dataflow, the steps' sources and the programs' commands. An
/// executor applies them again before it runs anything of a stored package.
pub fn structure(capsule: &SkillCapsule, files: &Files) -> Vec<Violation> {
    let mut violations = Vec::new();
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
        check_examples(&parsed, &text, examples, context, &mut violations);
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
            let literals: Vec<&str> = match value {
                Value::String(literal) => vec![literal.as_str()],
                Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
                _ => Vec::new(),
            };
            let mut leaked = false;
            for raw in literals {
                check_secrets(&format!("examples.bindings.{name}"), raw, &mut violations);
                if leaked {
                    continue;
                }
                let literal = normalized(raw);
                if literal.chars().count() >= EXAMPLE_LITERAL_CHARS && text.contains(&literal) {
                    violations.push(Violation::new(
                        "example_value_in_capsule",
                        format!("examples.bindings.{name}"),
                        "The procedure repeats this observed value; it describes the parameter, not the instance.",
                    ));
                    leaked = true;
                }
            }
        }
    }
    Report::of("gate1t", violations)
}
