use crate::{Result, RuntimeError, common, validation};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::{Path, PathBuf},
};
fn template(text: &str, name: &str) -> String {
    text.replace("{skill_name}", name)
        .replace("{skill_title}", &common::title(name))
}
pub fn init_skill(name: &str, parent: &Path, log: &mut String) -> Result<PathBuf> {
    // The original initializer deliberately leaves validation to quick-validate.
    let directory = common::absolute(parent)?.join(name);
    if directory.exists() {
        return Err(RuntimeError::conflict(format!(
            "Skill directory already exists: {}",
            directory.display()
        )));
    }
    std::fs::create_dir_all(&directory)?;
    log.push_str(&format!(
        "✅ Created skill directory: {}\n",
        directory.display()
    ));
    std::fs::write(
        directory.join("SKILL.md"),
        template(include_str!("../resources/skill-template.md"), name),
    )?;
    log.push_str("✅ Created SKILL.md\n");
    for sub in ["scripts", "references", "assets"] {
        std::fs::create_dir_all(directory.join(sub))?;
    }
    // A real native scaffold replaces the former executable Python example.
    let literal = serde_json::to_string(&format!("This is an example script for {name}"))?;
    let script = format!(
        "//! Example helper for {name}. Replace the example operation or delete it.\n//! Compile with rustc scripts/example.rs -o scripts/example.\nfn main() {{ println!(\"{{}}\", {literal}); }}\n"
    );
    std::fs::write(directory.join("scripts/example.rs"), script)?;
    log.push_str("✅ Created scripts/example.rs\n");
    std::fs::write(
        directory.join("references/api_reference.md"),
        template(include_str!("../resources/reference-template.md"), name),
    )?;
    log.push_str("✅ Created references/api_reference.md\n");
    std::fs::write(
        directory.join("assets/example_asset.txt"),
        include_str!("../resources/asset-template.txt"),
    )?;
    log.push_str("✅ Created assets/example_asset.txt\n");
    log.push_str(&format!("\n✅ Skill '{name}' initialized successfully at {}\n\nNext steps:\n1. Edit SKILL.md to complete the TODO items and update the description\n2. Customize or delete the example files in scripts/, references/, and assets/\n3. Run the validator when ready to check the skill structure\n",directory.display()));
    Ok(directory)
}
pub fn should_exclude(path: &Path, skill: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(skill) else {
        return true;
    };
    let components: Vec<_> = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect();
    components
        .iter()
        .any(|c| c == "__pycache__" || c == "node_modules")
        || components.first().is_some_and(|c| c == "evals")
        || path
            .file_name()
            .is_some_and(|s| s == ".DS_Store" || s.to_string_lossy().ends_with(".pyc"))
}
fn collect(directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for path in common::sorted(directory)? {
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            // Files may be symlinked only within the packaged skill image.
            if path.is_file() {
                files.push(path);
            }
            continue;
        }
        if meta.is_dir() {
            collect(&path, files)?;
        } else if meta.is_file() {
            files.push(path);
        }
    }
    Ok(())
}
pub fn package_skill(
    skill: &Path,
    output: Option<&Path>,
    log: &mut String,
) -> Result<Option<PathBuf>> {
    let skill = common::absolute(skill)?;
    if !skill.exists() {
        log.push_str(&format!(
            "❌ Error: Skill folder not found: {}\n",
            skill.display()
        ));
        return Ok(None);
    }
    if !skill.is_dir() {
        log.push_str(&format!(
            "❌ Error: Path is not a directory: {}\n",
            skill.display()
        ));
        return Ok(None);
    }
    if !skill.join("SKILL.md").exists() {
        log.push_str(&format!(
            "❌ Error: SKILL.md not found in {}\n",
            skill.display()
        ));
        return Ok(None);
    }
    log.push_str("🔍 Validating skill...\n");
    let (valid, message) = validation::validate_skill(&skill)?;
    if !valid {
        log.push_str(&format!("❌ Validation failed: {message}\n   Please fix the validation errors before packaging.\n"));
        return Ok(None);
    }
    log.push_str(&format!("✅ {message}\n\n"));
    let out = common::absolute(output.unwrap_or(Path::new(".")))?;
    std::fs::create_dir_all(&out)?;
    let name = skill
        .file_name()
        .ok_or_else(|| RuntimeError::invalid("Skill directory needs a name"))?
        .to_string_lossy();
    let destination = out.join(format!("{name}.skill"));
    let file = std::fs::File::create(&destination)?;
    let mut zip = zip::ZipWriter::new(file);
    let mut files = vec![];
    collect(&skill, &mut files)?;
    let canonical = infra_core::fs::canonicalize(&skill)?;
    for path in files {
        if should_exclude(&path, &skill) {
            log.push_str(&format!(
                "  Skipped: {}\n",
                path.strip_prefix(&skill).unwrap().display()
            ));
            continue;
        }
        if !infra_core::fs::canonicalize(&path)?.starts_with(&canonical) {
            return Err(RuntimeError::policy(
                "Archive member escapes skill directory",
            ));
        }
        let member = path
            .strip_prefix(skill.parent().unwrap())
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        #[cfg(unix)]
        let options = {
            use std::os::unix::fs::PermissionsExt;
            options.unix_permissions(std::fs::metadata(&path)?.permissions().mode())
        };
        zip.start_file(&member, options)
            .map_err(|e| RuntimeError::invalid(e.to_string()))?;
        zip.write_all(&std::fs::read(&path)?)?;
        log.push_str(&format!("  Added: {member}\n"));
    }
    zip.finish()
        .map_err(|e| RuntimeError::invalid(e.to_string()))?;
    log.push_str(&format!(
        "\n✅ Successfully packaged skill to: {}\n",
        destination.display()
    ));
    Ok(Some(destination))
}
pub fn execute(operation: &str, v: &Value) -> Result<Value> {
    let mut log = String::new();
    match operation {
        "init-skill" => {
            let name = common::positional(v, "skill_name", 0)?;
            let parent = Path::new(common::required(v, "path")?);
            log.push_str(&format!(
                "🚀 Initializing skill: {name}\n   Location: {}\n\n",
                parent.display()
            ));
            match init_skill(&name, parent, &mut log) {
                Ok(path) => Ok(common::cli(0, log, "", json!({"path":path}))),
                Err(e) => {
                    log.push_str(&format!("❌ Error: {e}\n"));
                    Ok(common::cli(1, log, "", Value::Null))
                }
            }
        }
        "package-skill" => {
            let skill = common::positional(v, "skill_path", 0)?;
            let output = common::text(v, "output_dir")
                .or_else(|| v["_positionals"].get(1).and_then(Value::as_str));
            let result = package_skill(Path::new(&skill), output.map(Path::new), &mut log)?;
            Ok(common::cli(
                if result.is_some() { 0 } else { 1 },
                log,
                "",
                json!({"path":result}),
            ))
        }
        _ => Err(RuntimeError::invalid("Unknown scaffold operation")),
    }
}
