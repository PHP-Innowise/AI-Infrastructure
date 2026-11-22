//! What an agent is told about a capsule it drives itself.
//!
//! The agent-driven form is an evaluation arm, not a warm turn: the agent gets
//! the capsule's plan and calls each program through `skillcache-tool` on its
//! own, with no executor between them. Like the warm prompt it never carries
//! an observed value, a program's code or where the package is; unlike it, it
//! names the programs, their inputs and their outputs, because the agent now
//! chains them.
use crate::{
    Form, Step,
    capsule::{SkillCapsule, SourceRef},
    package::Package,
};

fn source(text: &mut String, source: Option<&SourceRef>) {
    if let Some(source) = source {
        text.push_str(&format!(" (source: `{}`", source.path));
        if let Some(section) = &source.section {
            text.push_str(&format!(", section \"{section}\""));
        }
        text.push(')');
    }
}

fn names(names: &[String]) -> String {
    if names.is_empty() {
        "none".into()
    } else {
        names
            .iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn step(text: &mut String, capsule: &SkillCapsule, number: usize, step: &Step) {
    text.push_str(&format!("{number}. "));
    match step {
        Step::Instruction {
            text: instruction,
            source: from,
            produces,
        } => {
            text.push_str(&format!("Do yourself: {instruction}"));
            source(text, from.as_ref());
            if !produces.is_empty() {
                text.push_str(&format!(" Work out: {}.", names(produces)));
            }
        }
        Step::Inspection {
            text: instruction,
            source: from,
        } => {
            text.push_str(&format!("Check yourself: {instruction}"));
            source(text, from.as_ref());
        }
        Step::Program { program } => {
            let Some(program) = capsule.program(program) else {
                text.push_str(&format!("Run program `{program}`.\n"));
                return;
            };
            text.push_str(&format!(
                "Run `skillcache-tool {} --input '{{...}}'`",
                program.id
            ));
            if let Some(description) = &program.description {
                text.push_str(&format!(": {description}"));
            }
            text.push_str(&format!(
                ". Inputs: {}. Outputs for later steps: {}.",
                names(&program.inputs),
                names(&program.outputs)
            ));
            if program.read_only {
                text.push_str(" It changes nothing.");
            }
        }
    }
    text.push('\n');
}

/// The prompt block of the agent-driven form: what the capsule is for, the
/// values to take from the request, every step of the plan and its checks in
/// order, and how to call a program. `skill_path` is the source skill the
/// agent falls back on. A textual capsule has no programs to drive; its
/// written procedure is returned as it is.
pub fn render_agent_driven(package: &Package, skill_path: &str) -> String {
    let capsule = &package.capsule;
    let mut text = format!(
        "\n\nSkillCache capsule for this request: {} — {}\n",
        capsule.mu.name, capsule.mu.description
    );
    if capsule.form == Form::Textual {
        text.push_str(&format!(
            "This written procedure was compiled from the skill at `{skill_path}`. Follow it instead of reading the skill; read the skill only where the procedure does not cover what you meet.\n\n{}\n",
            package.markdown.as_deref().unwrap_or("").trim()
        ));
        return text;
    }
    text.push_str(&format!(
        "The skill at `{skill_path}` was compiled into the plan below. Do not read the skill first: follow the plan, and run each program step with `skillcache-tool`.\n"
    ));
    if !capsule.bindings.is_empty() {
        text.push_str("\nValues to take from the request:\n");
        for binding in &capsule.bindings {
            text.push_str(&format!("- `{}` ({})", binding.name, binding.kind.as_str()));
            if let Some(description) = &binding.description {
                text.push_str(&format!(": {description}"));
            }
            text.push('\n');
        }
    }
    text.push_str("\nPlan:\n");
    let mut number = 0;
    for planned in &capsule.steps {
        number += 1;
        step(&mut text, capsule, number, planned);
    }
    if !capsule.verification.is_empty() {
        text.push_str("\nThen verify:\n");
        for planned in &capsule.verification {
            number += 1;
            step(&mut text, capsule, number, planned);
        }
    }
    text.push_str(&format!(
        "\nHow to run a program:\n\
         - `skillcache-tool <program> --input '{{\"name\": value, ...}}'` with exactly the program's inputs: values from the request, values you worked out, or outputs of earlier programs. Each value is a string, a number, a boolean or a list of those, at most 4 KiB; name a file by its path, never pass its content.\n\
         - On success it prints one JSON object, `{{\"status\": \"complete\", \"result\": {{...}}}}`. Take the outputs later steps need from `result`, and answer from it.\n\
         - On failure it exits non-zero with the reason on standard error. Do that one step yourself with your normal tools, following the source skill at `{skill_path}`. Do not repair or re-run the program, then go on with the plan.\n\
         - Do not look for the capsule's files. If `skillcache-tool` itself cannot be started, read `{skill_path}` and follow the skill as usual.\n"
    ));
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{package, package::Files};
    use serde_json::json;

    fn package(root: &std::path::Path) -> Package {
        let capsule = json!({
            "schema_version": 1, "form": "hybrid",
            "mu": {"name": "pdf-totals", "description": "Totals of a folder of invoice PDFs"},
            "source": {"skills": [{"name": "pdf", "path": ".claude/skills/pdf/SKILL.md", "tree_sha256": "0".repeat(64)}]},
            "capabilities": {"network": "none", "runtimes": ["python3"]},
            "bindings": [{"name": "input_dir", "type": "path", "origin": "request", "description": "Folder with the invoices"}],
            "programs": [
                {"id": "extract", "description": "Reads the invoices", "entry": ["python3", "programs/extract/main.py"],
                 "inputs": ["input_dir", "quarter_files"], "outputs": ["rows_file"],
                 "check": ["python3", "programs/extract/main.py", "--check"], "timeout_seconds": 60, "read_only": false},
                {"id": "verify_csv", "entry": ["python3", "programs/verify_csv/main.py"],
                 "inputs": ["rows_file"], "outputs": [],
                 "check": ["python3", "programs/verify_csv/main.py", "--check"], "timeout_seconds": 60, "read_only": true}
            ],
            "steps": [
                {"kind": "instruction", "text": "Confirm which invoices belong to the quarter.",
                 "source": {"path": ".claude/skills/pdf/SKILL.md", "section": "Selecting files"}, "produces": ["quarter_files"]},
                {"kind": "program", "program": "extract"}
            ],
            "verification": [
                {"kind": "program", "program": "verify_csv"},
                {"kind": "inspection", "text": "Open the CSV and confirm one row per invoice."}
            ]
        });
        let parsed = SkillCapsule::parse(&serde_json::to_vec(&capsule).unwrap()).unwrap();
        let mut files = Files::new();
        files.insert(
            "programs/extract/main.py",
            b"SECRET_PROGRAM_SOURCE = 1\n".to_vec(),
        );
        files.insert(
            "programs/verify_csv/main.py",
            b"SECRET_PROGRAM_SOURCE = 2\n".to_vec(),
        );
        let warm = root.join("warm");
        package::write_warm(&warm, &parsed, None, &files).unwrap();
        package::load(&warm).unwrap()
    }

    #[test]
    fn the_agent_driven_prompt_names_the_plan_and_the_tool_only() {
        let root = tempfile::tempdir().unwrap();
        let stored = package(root.path());
        let text = render_agent_driven(&stored, ".claude/skills/pdf/SKILL.md");
        for expected in [
            "pdf-totals — Totals of a folder of invoice PDFs",
            "`input_dir` (path): Folder with the invoices",
            "1. Do yourself: Confirm which invoices belong to the quarter. (source: `.claude/skills/pdf/SKILL.md`, section \"Selecting files\") Work out: `quarter_files`.",
            "2. Run `skillcache-tool extract --input '{...}'`: Reads the invoices. Inputs: `input_dir`, `quarter_files`. Outputs for later steps: `rows_file`.",
            "Then verify:",
            "3. Run `skillcache-tool verify_csv --input '{...}'`",
            "It changes nothing.",
            "4. Check yourself: Open the CSV and confirm one row per invoice.",
            "following the source skill at `.claude/skills/pdf/SKILL.md`",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
        }
        for withheld in [
            "skillcache-run",
            "SECRET_PROGRAM_SOURCE",
            "main.py",
            "python3",
            root.path().to_str().unwrap(),
        ] {
            assert!(!text.contains(withheld), "shows {withheld:?}:\n{text}");
        }
        package::release(root.path());
    }

    #[test]
    fn a_textual_capsule_is_its_procedure() {
        let root = tempfile::tempdir().unwrap();
        let manifest = json!({
            "schema_version": 1, "form": "textual",
            "mu": {"name": "pdf-totals", "description": "Totals"},
            "source": {"skills": [{"name": "pdf", "path": ".claude/skills/pdf/SKILL.md", "tree_sha256": "0".repeat(64)}]}
        });
        let parsed = SkillCapsule::parse(&serde_json::to_vec(&manifest).unwrap()).unwrap();
        let warm = root.path().join("warm");
        package::write_warm(
            &warm,
            &parsed,
            Some(b"# Totals\n\n1. Read.\n"),
            &Files::new(),
        )
        .unwrap();
        let text = render_agent_driven(&package::load(&warm).unwrap(), "s/SKILL.md");
        assert!(text.contains("# Totals\n\n1. Read."));
        assert!(!text.contains("skillcache-tool"));
        package::release(root.path());
    }
}
