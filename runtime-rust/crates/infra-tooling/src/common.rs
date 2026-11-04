use infra_core::{Result, RuntimeError, fs as safe, git};
use regex::Regex;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    path::{Component, Path, PathBuf},
    time::Duration,
};
pub const EDITIONS: &[&str] = &[
    "Laravel",
    "Symfony",
    "PHP Core",
    "Cms/wordpress",
    "Infrastructure-Creator",
];
pub const PHP_EDITIONS: &[&str] = &["Laravel", "Symfony", "PHP Core", "Cms/wordpress"];
pub fn sha256(bytes: &[u8]) -> String {
    infra_core::query::sha256(bytes)
}
pub fn git(root: &Path, args: &[&str], input: Option<Vec<u8>>) -> Result<git::GitOutput> {
    git::run(
        root,
        &args.iter().map(OsString::from).collect::<Vec<_>>(),
        input,
        Duration::from_secs(30),
        64 * 1024 * 1024,
    )
}
pub fn git_ok(root: &Path, args: &[&str], input: Option<Vec<u8>>) -> Result<Vec<u8>> {
    let output = git(root, args, input)?;
    if output.code != 0 {
        return Err(RuntimeError::unavailable(format!(
            "Git operation failed: {}",
            args.first().copied().unwrap_or("unknown")
        )));
    }
    Ok(output.stdout)
}
pub fn tracked(root: &Path, specs: &[String]) -> Result<Vec<String>> {
    let mut args = vec![
        "ls-files".to_string(),
        "-z".into(),
        "--cached".into(),
        "--".into(),
    ];
    args.extend_from_slice(specs);
    let raw = git_ok(
        root,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        None,
    )?;
    if !raw.is_empty() && raw.last() != Some(&0) {
        return Err(RuntimeError::unavailable(
            "Incomplete tracked-file response",
        ));
    }
    let mut paths = BTreeSet::new();
    for bytes in raw
        .split(|byte| *byte == 0)
        .filter(|bytes| !bytes.is_empty())
    {
        let name = std::str::from_utf8(bytes)
            .map_err(|_| RuntimeError::invalid("Distribution path is not UTF-8"))?;
        relative(name)?;
        paths.insert(name.to_string());
    }
    Ok(paths.into_iter().collect())
}
pub fn relative(value: &str) -> Result<PathBuf> {
    // The refused value names the caller that built it, on any platform.
    let refused = || RuntimeError::policy(format!("Unsafe relative distribution path: {value:?}"));
    if value.is_empty() || value.contains(['\\', '\0', ':']) {
        return Err(refused());
    }
    let path = Path::new(value);
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(refused());
    }
    Ok(path.to_path_buf())
}
pub fn under(root: &Path, relative_name: &str) -> Result<PathBuf> {
    let path = root.join(relative(relative_name)?);
    safe::admit_descendant(root, &path)?;
    Ok(path)
}
pub fn read_json(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&safe::read_regular(path)?)?)
}
pub fn text(bytes: Vec<u8>) -> Result<String> {
    String::from_utf8(bytes).map_err(|_| RuntimeError::invalid("Distribution text is not UTF-8"))
}
pub fn walk(root: &Path) -> Result<Vec<String>> {
    fn visit(root: &Path, dir: &Path, found: &mut Vec<String>) -> Result<()> {
        let mut entries = fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let image = fs::symlink_metadata(&path)?;
            if safe::linked(&image) {
                return Err(RuntimeError::policy("Linked distribution entry is refused"));
            }
            if image.is_dir() {
                if entry.file_name() == "__pycache__" || entry.file_name() == ".git" {
                    continue;
                }
                visit(root, &path, found)?;
            } else if image.is_file() {
                found.push(
                    path.strip_prefix(root)
                        .map_err(|_| RuntimeError::policy("Distribution walk escaped its root"))?
                        .to_str()
                        .ok_or_else(|| RuntimeError::invalid("Distribution path is not UTF-8"))?
                        .replace('\\', "/"),
                );
            } else {
                return Err(RuntimeError::policy(
                    "Nonregular distribution entry is refused",
                ));
            }
        }
        Ok(())
    }
    if !safe::admit_directory(root)? {
        return Err(RuntimeError::not_found("Distribution directory is absent"));
    }
    let mut found = Vec::new();
    visit(root, root, &mut found)?;
    Ok(found)
}
fn compile_glob(pattern: &str, slash_sensitive: bool) -> Result<Regex> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut index = 0;
    let mut expression = String::from("^");
    while index < chars.len() {
        match chars[index] {
            '*' if index + 1 < chars.len() && chars[index + 1] == '*' => {
                index += 1;
                if index + 1 < chars.len() && chars[index + 1] == '/' {
                    index += 1;
                    expression.push_str("(?:.*/)?");
                } else {
                    expression.push_str(".*");
                }
            }
            '*' => expression.push_str(if slash_sensitive { "[^/]*" } else { ".*" }),
            '?' => expression.push_str(if slash_sensitive { "[^/]" } else { "." }),
            '[' => {
                let mut end = index + 1;
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
                    expression.push_str("\\[");
                } else {
                    expression.push('[');
                    let mut start = index + 1;
                    if chars.get(start) == Some(&'!') {
                        expression.push('^');
                        start += 1;
                    } else if chars.get(start) == Some(&'^') {
                        expression.push_str("\\^");
                        start += 1;
                    }
                    for character in &chars[start..end] {
                        if *character == '\\' || *character == '[' || *character == ']' {
                            expression.push('\\');
                        }
                        expression.push(*character);
                    }
                    expression.push(']');
                    index = end;
                }
            }
            value => expression.push_str(&regex::escape(&value.to_string())),
        }
        index += 1;
    }
    expression.push('$');
    Regex::new(&expression).map_err(|_| RuntimeError::invalid("Invalid glob expression"))
}
pub fn matches(pattern: &str, name: &str, slash_sensitive: bool) -> Result<bool> {
    Ok(compile_glob(pattern, slash_sensitive)?.is_match(name))
}
/// A command's finite configuration patterns, compiled only when reached.
/// Keeping this state local preserves ordered matching and late glob errors.
#[derive(Default)]
pub(crate) struct Globs {
    sensitive: BTreeMap<String, Regex>,
    loose: BTreeMap<String, Regex>,
}
impl Globs {
    pub(crate) fn matches(
        &mut self,
        pattern: &str,
        name: &str,
        slash_sensitive: bool,
    ) -> Result<bool> {
        let cache = if slash_sensitive {
            &mut self.sensitive
        } else {
            &mut self.loose
        };
        if let Some(compiled) = cache.get(pattern) {
            return Ok(compiled.is_match(name));
        }
        let compiled = compile_glob(pattern, slash_sensitive)?;
        let matched = compiled.is_match(name);
        cache.insert(pattern.into(), compiled);
        Ok(matched)
    }
}
pub fn ignored(path: &str) -> bool {
    path.split('/')
        .any(|part| part == "__pycache__" || part == ".DS_Store")
        || path.ends_with(".pyc")
}
pub fn json_bytes(value: &Value) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}
pub fn write(root: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    let path = under(root, name)?;
    safe::atomic_write(&path, bytes)
}
pub fn edition(value: &str) -> Result<&'static str> {
    let name = match value {
        "laravel" => "Laravel",
        "symfony" => "Symfony",
        "php-core" => "PHP Core",
        "wordpress" | "WordPress" => "Cms/wordpress",
        "infrastructure-creator" => "Infrastructure-Creator",
        name => name,
    };
    EDITIONS
        .iter()
        .copied()
        .find(|known| *known == name)
        .ok_or_else(|| RuntimeError::invalid("Unknown accelerator edition"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_globs_preserve_slash_and_bracket_semantics_on_repeated_calls() {
        let mut globs = Globs::default();
        for _ in 0..2 {
            for (pattern, name, sensitive, expected) in [
                ("src/**/*.php", "src/A.php", true, true),
                ("src/**/*.php", "src/deep/A.php", true, true),
                ("src/*.php", "src/deep/A.php", true, false),
                ("src/*.php", "src/deep/A.php", false, true),
                ("file?.md", "filex.md", true, true),
                ("file?.md", "file/.md", true, false),
                ("file?.md", "file/.md", false, true),
                ("[!a-c].md", "d.md", true, true),
                ("[!a-c].md", "b.md", true, false),
                ("file[0-9].md", "file2.md", true, true),
                ("literal[", "literal[", true, true),
            ] {
                assert_eq!(
                    globs.matches(pattern, name, sensitive).unwrap(),
                    expected,
                    "pattern={pattern}, name={name}, slash_sensitive={sensitive}"
                );
                assert_eq!(matches(pattern, name, sensitive).unwrap(), expected);
            }
        }
    }

    #[test]
    fn invalid_globs_keep_the_original_error_on_repeated_calls() {
        let mut globs = Globs::default();
        for sensitive in [true, false, true, false] {
            let error = globs.matches("[z-a]", "file", sensitive).unwrap_err();
            assert_eq!(error.kind, infra_core::ErrorKind::InvalidInput);
            assert_eq!(error.message, "Invalid glob expression");
        }
    }
}
