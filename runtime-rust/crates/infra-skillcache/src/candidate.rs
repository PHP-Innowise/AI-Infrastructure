//! A compiler's structured answer, turned into what the gates check.
//!
//! The compiler returns values; the host writes every file. What the host
//! knows better than the compiler is filled in here: the schema version, the
//! form that was asked for, and the identity of each source skill as it was
//! when the run was observed.
use crate::{
    capsule::{CapsuleKind, Examples, Form, SCHEMA_VERSION, SkillCapsule, Violation},
    package::Files,
};
use serde_json::{Value, json};

/// A skill of the observed run: what `source.skills` may name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KnownSkill {
    pub name: String,
    pub path: String,
    pub tree_sha256: String,
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub capsule: SkillCapsule,
    /// `capsule.json` in its stored spelling.
    pub capsule_bytes: Vec<u8>,
    pub files: Files,
    pub examples: Option<Examples>,
    pub markdown: Option<String>,
}

/// Reads a compiler's answer for the form that was asked of it. Every
/// refusal is reported; a candidate is returned only when there was none.
pub fn from_answer(
    answer: &Value,
    skills: &[KnownSkill],
    form: Form,
) -> Result<Candidate, Vec<Violation>> {
    from_answer_with_kind(answer, skills, CapsuleKind::Skill, None, form)
}

/// Reads a compiler's answer for a skillless read-only tool-chain capsule bound to `tree_sha256`.
pub fn from_tool_chain_answer(
    answer: &Value,
    tree_sha256: &str,
    form: Form,
) -> Result<Candidate, Vec<Violation>> {
    from_answer_with_kind(answer, &[], CapsuleKind::ToolChain, Some(tree_sha256), form)
}

/// Converts a verified candidate into a portable edition template tagged with `edition`.
pub fn as_portable_template(candidate: &Candidate, edition: &str) -> Candidate {
    let mut capsule = candidate.capsule.clone();
    capsule.kind = CapsuleKind::Template;
    capsule.source_edition = Some(edition.to_string());
    Candidate {
        capsule_bytes: capsule.to_bytes(),
        capsule,
        files: candidate.files.clone(),
        examples: candidate.examples.clone(),
        markdown: candidate.markdown.clone(),
    }
}

fn from_answer_with_kind(
    answer: &Value,
    skills: &[KnownSkill],
    kind: CapsuleKind,
    tool_chain_tree_sha: Option<&str>,
    form: Form,
) -> Result<Candidate, Vec<Violation>> {
    let mut violations = Vec::new();
    let Some(members) = answer.as_object() else {
        return Err(vec![Violation::new(
            "invalid_answer",
            "",
            "The answer is one JSON object.",
        )]);
    };
    for key in members.keys() {
        if !["form", "capsule", "files", "examples", "markdown"].contains(&key.as_str()) {
            violations.push(Violation::new(
                "invalid_answer",
                key.clone(),
                format!("The answer has no member `{key}`."),
            ));
        }
    }
    if answer["form"].as_str() != Some(form.as_str()) {
        violations.push(Violation::new(
            "wrong_form",
            "form",
            format!("This round asks for the {} form.", form.as_str()),
        ));
    }
    let mut capsule = answer["capsule"].clone();
    if !capsule.is_object() {
        violations.push(Violation::new(
            "invalid_answer",
            "capsule",
            "The answer carries the capsule as an object.",
        ));
        return Err(violations);
    }
    capsule["schema_version"] = SCHEMA_VERSION.into();
    capsule["form"] = form.as_str().into();
    if kind == CapsuleKind::Skill {
        if let Some(obj) = capsule.as_object_mut() {
            obj.remove("kind");
        }
    } else {
        capsule["kind"] = kind.as_str().into();
    }
    if kind == CapsuleKind::ToolChain {
        if !capsule["source"].is_object() {
            capsule["source"] = json!({"skills": []});
        }
        capsule["source"]["skills"] = json!([]);
        if let Some(tree_sha) = tool_chain_tree_sha {
            capsule["source"]["tool_chain"] = json!({"tree_sha256": tree_sha});
        }
    } else {
        // The compiler names the skills; their identities are the observed ones.
        match capsule["source"]["skills"].as_array_mut() {
            Some(named) if !named.is_empty() => {
                for (index, skill) in named.iter_mut().enumerate() {
                    let known = skill["path"]
                        .as_str()
                        .and_then(|path| skills.iter().find(|known| known.path == path));
                    match known {
                        Some(known) => {
                            skill["name"] = known.name.clone().into();
                            skill["tree_sha256"] = known.tree_sha256.clone().into();
                        }
                        None => {
                            // Reported once, as what it is; the reader below is
                            // not also told that the identity is missing.
                            skill["tree_sha256"] = "0".repeat(64).into();
                            if !skill["name"].is_string() {
                                skill["name"] = "unknown".into();
                            }
                            violations.push(Violation::new(
                                "source_skill_unknown",
                                format!("capsule.source.skills[{index}].path"),
                                format!(
                                    "The observed run used: {}. Name one of these paths.",
                                    skills
                                        .iter()
                                        .map(|known| known.path.as_str())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ),
                            ));
                        }
                    }
                }
            }
            _ => violations.push(Violation::new(
                "missing_field",
                "capsule.source.skills",
                "The capsule names the skill it was compiled from.",
            )),
        }
    }
    let files = Files::from_response(&answer["files"]);
    let mut examples = None;
    let mut markdown = None;
    match form {
        Form::Hybrid => {
            if !answer["markdown"].is_null() {
                violations.push(Violation::new(
                    "invalid_answer",
                    "markdown",
                    "A hybrid capsule has no written procedure.",
                ));
            }
            match &answer["examples"] {
                Value::Null => violations.push(Violation::new(
                    "example_missing",
                    "examples",
                    "The answer carries the observed values the replay answers with.",
                )),
                observed => match Examples::from_value(observed) {
                    Ok(observed) => examples = Some(observed),
                    Err(found) => violations.extend(found),
                },
            }
        }
        Form::Textual => {
            if !files.is_empty()
                || !answer["files"].is_null() && answer["files"] != Value::Array(vec![])
            {
                violations.push(Violation::new(
                    "invalid_answer",
                    "files",
                    "A textual capsule has no programs.",
                ));
            }
            if !answer["examples"].is_null() {
                violations.push(Violation::new(
                    "examples_in_capsule",
                    "examples",
                    "A textual capsule carries no observed values.",
                ));
            }
            match answer["markdown"].as_str() {
                Some(text) if !text.trim().is_empty() => markdown = Some(text.to_string()),
                _ => violations.push(Violation::new(
                    "missing_field",
                    "markdown",
                    "The textual form is its written procedure.",
                )),
            }
        }
    }
    let parsed = match SkillCapsule::from_value(&capsule) {
        Ok(parsed) => Some(parsed),
        Err(found) => {
            violations.extend(found);
            None
        }
    };
    match parsed {
        Some(capsule) if violations.is_empty() => Ok(Candidate {
            capsule_bytes: capsule.to_bytes(),
            capsule,
            files,
            examples,
            markdown,
        }),
        _ => Err(violations),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn skills() -> Vec<KnownSkill> {
        vec![KnownSkill {
            name: "pdf".into(),
            path: ".claude/skills/pdf/SKILL.md".into(),
            tree_sha256: "a".repeat(64),
        }]
    }
    fn answer() -> Value {
        json!({
            "form": "hybrid",
            "capsule": {
                "mu": {"name": "pdf-totals", "description": "Totals of invoice PDFs"},
                "source": {"skills": [{"name": "whatever", "path": ".claude/skills/pdf/SKILL.md"}]},
                "capabilities": {"network": "none", "runtimes": ["python3"]},
                "bindings": [{"name": "input_dir", "type": "path", "origin": "request"}],
                "programs": [{"id": "extract", "entry": ["python3", "programs/extract/main.py"],
                    "inputs": ["input_dir"], "outputs": [], "check": ["python3", "programs/extract/main.py", "--check"],
                    "timeout_seconds": 60, "read_only": false}],
                "steps": [{"kind": "program", "program": "extract"}],
                "verification": []
            },
            "files": [{"path": "programs/extract/main.py", "content": "print(1)\n"}],
            "examples": {"bindings": {"input_dir": "invoices/"}, "steps": {}}
        })
    }
    fn codes(answer: &Value, form: Form) -> Vec<&'static str> {
        from_answer(answer, &skills(), form)
            .err()
            .unwrap_or_default()
            .into_iter()
            .map(|violation| violation.code)
            .collect()
    }

    #[test]
    fn the_host_fills_in_what_it_knows() {
        let candidate = from_answer(&answer(), &skills(), Form::Hybrid).unwrap();
        // The observed identity, whatever name the compiler gave the skill.
        assert_eq!(candidate.capsule.skills[0].name, "pdf");
        assert_eq!(candidate.capsule.skills[0].tree_sha256, "a".repeat(64));
        assert_eq!(candidate.files.len(), 1);
        assert_eq!(
            candidate.examples.unwrap().bindings["input_dir"],
            "invoices/"
        );
        let stored: Value = serde_json::from_slice(&candidate.capsule_bytes).unwrap();
        assert_eq!(stored["schema_version"], 1);
        assert_eq!(stored["form"], "hybrid");
        // What the compiler says about version and form is not taken from it.
        let mut claimed = answer();
        claimed["capsule"]["schema_version"] = 7.into();
        claimed["capsule"]["form"] = "textual".into();
        assert!(from_answer(&claimed, &skills(), Form::Hybrid).is_ok());
    }

    #[test]
    fn an_answer_outside_its_round_is_refused() {
        assert_eq!(codes(&answer(), Form::Textual)[0], "wrong_form");
        let mut other = answer();
        other["capsule"]["source"]["skills"][0]["path"] = ".claude/skills/other/SKILL.md".into();
        assert_eq!(codes(&other, Form::Hybrid), ["source_skill_unknown"]);
        let mut none = answer();
        none["capsule"]["source"] = json!({"skills": []});
        assert!(codes(&none, Form::Hybrid).contains(&"missing_field"));
        let mut without = answer();
        without.as_object_mut().unwrap().remove("examples");
        assert_eq!(codes(&without, Form::Hybrid), ["example_missing"]);
        let mut extra = answer();
        extra["notes"] = "x".into();
        extra["markdown"] = "# x".into();
        assert_eq!(
            codes(&extra, Form::Hybrid),
            ["invalid_answer", "invalid_answer"]
        );
        assert_eq!(codes(&json!([]), Form::Hybrid), ["invalid_answer"]);
        assert_eq!(
            codes(&json!({"form": "hybrid"}), Form::Hybrid),
            ["invalid_answer"]
        );
        // A structural refusal of the capsule itself comes through with its code.
        let mut broken = answer();
        broken["capsule"]["programs"][0]["timeout_seconds"] = 0.into();
        assert_eq!(codes(&broken, Form::Hybrid), ["invalid_value"]);
    }

    #[test]
    fn the_textual_form_is_its_markdown_alone() {
        let textual = json!({
            "form": "textual",
            "capsule": {
                "mu": {"name": "pdf-totals", "description": "Totals of invoice PDFs"},
                "source": {"skills": [{"name": "pdf", "path": ".claude/skills/pdf/SKILL.md"}]}
            },
            "markdown": "# Totals\n\n1. Read.\n"
        });
        let candidate = from_answer(&textual, &skills(), Form::Textual).unwrap();
        assert_eq!(candidate.capsule.form, Form::Textual);
        assert_eq!(
            candidate.markdown.as_deref(),
            Some("# Totals\n\n1. Read.\n")
        );
        assert!(candidate.examples.is_none() && candidate.files.is_empty());
        let mut with = textual.clone();
        with["files"] = json!([{"path": "programs/x/main.py", "content": "x"}]);
        with["examples"] = json!({"bindings": {}});
        assert_eq!(
            codes(&with, Form::Textual),
            ["invalid_answer", "examples_in_capsule"]
        );
        let mut empty = textual;
        empty["markdown"] = "  ".into();
        assert_eq!(codes(&empty, Form::Textual), ["missing_field"]);
    }

    #[test]
    fn tool_chain_and_template_candidates_round_trip() {
        let mut chain = answer();
        chain["capsule"]["source"] = json!({"skills": []});
        chain["capsule"]["programs"][0]["read_only"] = true.into();
        let tree_sha = "d".repeat(64);
        let built = from_tool_chain_answer(&chain, &tree_sha, Form::Hybrid).unwrap();
        assert_eq!(built.capsule.kind, CapsuleKind::ToolChain);
        assert!(built.capsule.skills.is_empty());
        assert_eq!(
            built.capsule.tool_chain_sha256.as_deref(),
            Some(tree_sha.as_str())
        );

        let skill_cand = from_answer(&answer(), &skills(), Form::Hybrid).unwrap();
        let template = as_portable_template(&skill_cand, "Laravel");
        assert_eq!(template.capsule.kind, CapsuleKind::Template);
        assert_eq!(template.capsule.source_edition.as_deref(), Some("Laravel"));
    }
}
