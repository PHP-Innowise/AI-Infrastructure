use crate::{
    common,
    file_modes::FileModes,
    manifests::{MirrorClass, MirrorManifest, MirrorSpec},
};
use infra_core::{Result, RuntimeError, fs as safe};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
// These are the exact ignored outputs materialized for memory-seed assets.
// They still participate in byte/mode parity, but cannot affect tracked attributes.
fn assembled_seed_output(relative: &str) -> bool {
    [
        "/memory-seed/assets/bin/infra-engine",
        "/memory-seed/assets/bin/infra-engine.exe",
        "/memory-seed/assets/native-artifacts.json",
    ]
    .iter()
    .any(|suffix| relative.ends_with(suffix))
}
fn split(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---\n")?;
    Some((&rest[..end + 1], &rest[end + 5..]))
}
fn replacements(mut text: String, spec: &MirrorSpec) -> String {
    for (old, new) in &spec.replacements {
        text = text.replace(old, new);
    }
    text
}
pub fn transform(relative: &str, text: &str, spec: &MirrorSpec) -> Result<String> {
    if spec.transform == "copy" {
        return Ok(replacements(text.into(), spec));
    }
    let Some((front, body)) = split(text) else {
        return Ok(replacements(text.into(), spec));
    };
    let body = replacements(body.into(), spec);
    if spec.transform == "cursor-agent" {
        let kept = front
            .lines()
            .filter(|line| {
                !["model:", "invokes:", "phase:"]
                    .iter()
                    .any(|key| line.starts_with(key))
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(format!("---\n{kept}\n---\n{body}"));
    }
    if front.lines().any(|line| line.starts_with("name:")) {
        return Ok(format!("---\n{front}---\n{body}"));
    }
    let name = relative.rsplit('/').next().unwrap_or(relative);
    let stem = name.strip_suffix(".md").unwrap_or(name);
    let description = if let Some(value) = spec.description_overrides.get(name) {
        value.clone()
    } else {
        body.lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
            .ok_or_else(|| {
                RuntimeError::invalid(format!("{relative}: cannot derive a command description"))
            })?
            .into()
    };
    let description = if spec.quote_description {
        format!("\"{description}\"")
    } else {
        description
    };
    Ok(format!(
        "---\nname: {stem}\ndescription: {description}\n---\n{body}"
    ))
}
fn skipped(class: &MirrorClass, spec: &MirrorSpec, framework: Option<&str>, name: &str) -> bool {
    class
        .skip
        .iter()
        .chain(spec.skip.iter())
        .chain(
            framework
                .and_then(|value| class.skip_by_framework.get(value))
                .into_iter()
                .flatten(),
        )
        .any(|entry| {
            if entry.ends_with('/') {
                name.starts_with(entry)
            } else {
                name == entry
            }
        })
}
pub fn process(root: &Path, edition: &str, write: bool) -> Result<Value> {
    let edition = common::edition(edition)?;
    let base = root.join(edition);
    let rules = MirrorManifest::load(&base)?;
    let framework_path = base.join("project-brain/config/runtime.json");
    let framework = if framework_path.is_file() {
        common::read_json(&framework_path)?["framework"]
            .as_str()
            .map(str::to_string)
    } else {
        None
    };
    let mut modes = FileModes::read(&base)?;
    let mut findings = Vec::<String>::new();
    let mut changed = Vec::<String>::new();
    let mut generated = BTreeSet::new();
    for class in &rules.classes {
        let canonical = base.join(&class.canonical);
        let names = if let Some(only) = &class.only {
            only.clone()
        } else {
            common::walk(&canonical)?
        };
        for mirror in class.mirrors.keys() {
            let spec = &class.mirrors[mirror];
            let mut expected = BTreeMap::new();
            for name in &names {
                if skipped(class, spec, framework.as_deref(), name) || common::ignored(name) {
                    continue;
                }
                let source = common::under(&canonical, name)?;
                if !source.is_file() {
                    findings.push(format!(
                        "{edition}: {} is listed but absent",
                        source.display()
                    ));
                    continue;
                }
                let bytes = safe::read_regular(&source)?;
                let output = if let Ok(text) = std::str::from_utf8(&bytes) {
                    transform(name, text, spec)?.into_bytes()
                } else if spec.transform == "copy" && spec.replacements.is_empty() {
                    bytes
                } else {
                    return Err(RuntimeError::invalid(
                        "Text mirror transformation received binary data",
                    ));
                };
                expected.insert(name.clone(), output);
                let relative = format!("{mirror}/{name}");
                if !assembled_seed_output(&relative) {
                    generated.insert(relative);
                }
            }
            for (name, bytes) in &expected {
                let relative = format!("{mirror}/{name}");
                let target = common::under(&base, &relative)?;
                let actual = if target.exists() {
                    Some(safe::read_regular(&target)?)
                } else {
                    None
                };
                let executable = modes.executable(&format!("{}/{name}", class.canonical))?;
                let mode_diff = if let Some(executable) = executable {
                    actual.is_some()
                        && modes
                            .executable(&relative)?
                            .is_some_and(|mode| mode != executable)
                } else {
                    false
                };
                if actual.as_deref() != Some(bytes.as_slice()) || mode_diff {
                    if write {
                        common::write(&base, &relative, bytes)?;
                        if let Some(executable) = executable {
                            modes.set(&relative, executable)?;
                        }
                        changed.push(relative);
                    } else {
                        findings.push(format!(
                            "{edition}/{relative}: mirror content or mode differs"
                        ));
                    }
                }
            }
            let directory = base.join(mirror);
            if directory.is_dir() {
                for name in common::walk(&directory)? {
                    if common::ignored(&name)
                        || skipped(class, spec, framework.as_deref(), &name)
                        || expected.contains_key(&name)
                    {
                        continue;
                    }
                    if class
                        .only
                        .as_ref()
                        .is_some_and(|only| !only.contains(&name))
                    {
                        continue;
                    }
                    let relative = format!("{mirror}/{name}");
                    if write {
                        std::fs::remove_file(common::under(&base, &relative)?)?;
                        changed.push(relative);
                    } else {
                        findings.push(format!("{edition}/{relative}: stray mirror file"));
                    }
                }
            }
        }
    }
    let mut attributes =
        String::from("# Generated by the native mirror engine; do not edit by hand.\n");
    for path in generated {
        attributes.push_str(&format!(
            "{} -diff linguist-generated=true\n",
            path.replace(' ', "?")
        ));
    }
    let current = base.join(".gitattributes");
    if !current.is_file() || safe::read_regular(&current)? != attributes.as_bytes() {
        if write {
            common::write(&base, ".gitattributes", attributes.as_bytes())?;
            changed.push(".gitattributes".into());
        } else {
            findings.push(format!(
                "{edition}/.gitattributes: generated attributes differ"
            ));
        }
    }
    if write {
        modes.flush()?;
    }
    Ok(
        json!({"edition":edition,"write":write,"changed":changed,"findings":findings,"passed":findings.is_empty()}),
    )
}

#[cfg(test)]
mod assembled_attribute_tests {
    use super::*;
    use std::{fs, path::PathBuf};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("infra-mirror-attributes-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            let root = infra_core::fs::canonicalize(root).unwrap();
            common::git_ok(&root, &["-c", "init.templateDir=", "init", "--quiet"], None).unwrap();
            #[cfg(unix)]
            common::git_ok(&root, &["config", "core.fileMode", "true"], None).unwrap();
            Self(root)
        }
        fn write(&self, relative: &str, bytes: &[u8]) {
            let destination = self.0.join("PHP Core").join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::write(destination, bytes).unwrap();
        }
        fn process(&self, write: bool) {
            let report = process(&self.0, "PHP Core", write).unwrap();
            assert_eq!(report["passed"], true, "{report}");
        }
        fn attributes(&self) -> Vec<u8> {
            fs::read(self.0.join("PHP Core/.gitattributes")).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn assembly_copies_exact_outputs_without_changing_clean_checkout_attributes() {
        let fixture = Fixture::new();
        fixture.write(
            "memory-bank/config/mirror-rules.json",
            &common::json_bytes(&json!({
                "version":1,
                "classes":[{
                    "name":"skills",
                    "canonical":".agents/skills",
                    "mirrors":{
                        ".claude/skills":{"transform":"copy"},
                        ".cursor/skills":{"transform":"copy"}
                    }
                }]
            }))
            .unwrap(),
        );
        fixture.write(".agents/skills/ordinary/SKILL.md", b"# Ordinary skill\n");
        // Similar names outside memory-seed remain normal tracked mirror inputs.
        fixture.write(
            ".agents/skills/ordinary/assets/bin/infra-engine",
            b"ordinary helper\n",
        );
        fixture.write(".agents/skills/ordinary/native-artifacts.json", b"{}\n");
        fixture.process(true);
        let before = fixture.attributes();
        let text = std::str::from_utf8(&before).unwrap();
        assert!(text.contains(".cursor/skills/ordinary/assets/bin/infra-engine -diff"));
        assert!(text.contains(".claude/skills/ordinary/native-artifacts.json -diff"));

        // This checks binary copying, not capability admission. Assembly and
        // installer integration tests separately admit their actual native engine.
        let outputs: [(&str, &[u8]); 3] = [
            ("bin/infra-engine", b"\x7fELF\0\xffmirror image\n"),
            ("bin/infra-engine.exe", b"MZ\0\xffmirror image\n"),
            ("native-artifacts.json", b"{\"engine\":\"infra-engine\"}\n"),
        ];
        for (name, bytes) in outputs {
            let relative = format!(".agents/skills/memory-seed/assets/{name}");
            fixture.write(&relative, bytes);
            #[cfg(unix)]
            if name.starts_with("bin/") {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(
                    fixture.0.join("PHP Core").join(&relative),
                    fs::Permissions::from_mode(0o755),
                )
                .unwrap();
            }
        }
        fixture.process(true);
        assert_eq!(
            fixture.attributes(),
            before,
            "assembly must not alter tracked attributes"
        );
        for mirror in [".claude/skills", ".cursor/skills"] {
            for (name, bytes) in outputs {
                let destination = fixture
                    .0
                    .join("PHP Core")
                    .join(format!("{mirror}/memory-seed/assets/{name}"));
                assert_eq!(fs::read(&destination).unwrap(), bytes);
                #[cfg(unix)]
                if name.starts_with("bin/") {
                    use std::os::unix::fs::PermissionsExt;
                    // FileModes preserves owner execution. Private publication
                    // need not grant group or other execute permissions.
                    assert_eq!(
                        fs::metadata(destination).unwrap().permissions().mode() & 0o100,
                        0o100
                    );
                }
            }
        }
        fixture.process(false);

        // A clean checkout omits all ignored canonical and mirrored artifacts.
        for prefix in [".agents/skills", ".claude/skills", ".cursor/skills"] {
            for (name, _) in outputs {
                fs::remove_file(
                    fixture
                        .0
                        .join("PHP Core")
                        .join(format!("{prefix}/memory-seed/assets/{name}")),
                )
                .unwrap();
            }
        }
        fixture.process(false);
        assert_eq!(fixture.attributes(), before);
    }
}
