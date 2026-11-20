//! Canonical workspace code file extensions, ignored directory filters, and test-path classifiers.

pub(crate) const CODE_EXTENSIONS: &[&str] = &[
    "php", "phtml", "inc", "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts", "py", "pyi",
    "go", "java", "kt", "kts", "cs", "rb", "swift", "scala", "sc",
];

pub(crate) const TEST_FOLDERS: &[&str] = &[
    "test",
    "tests",
    "__tests__",
    "spec",
    "testdata",
    "fixtures",
    "__fixtures__",
    "__mocks__",
    "e2e",
    "cypress",
];

pub(crate) const IGNORED_CODE_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "graphify-out",
    "node_modules",
    "vendor",
    "target",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    ".nuxt",
    ".cargo",
    ".cache",
    ".pytest_cache",
    ".mypy_cache",
    ".idea",
    ".vscode",
    ".c2p",
];

pub(crate) fn is_blade_template(path: &str) -> bool {
    let leaf = path.rsplit('/').next().unwrap_or(path);
    leaf.to_ascii_lowercase().ends_with(".blade.php")
}

pub(crate) fn is_minified_js(path: &str) -> bool {
    let fname = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    fname.ends_with(".min.js") || fname.ends_with(".min.mjs") || fname.ends_with(".min.cjs")
}

pub(crate) fn has_code_extension(path: &str) -> bool {
    if is_blade_template(path) || is_minified_js(path) {
        return false;
    }
    let leaf = path.rsplit('/').next().unwrap_or(path);
    leaf.rsplit_once('.')
        .is_some_and(|(_, ext)| CODE_EXTENSIONS.iter().any(|k| ext.eq_ignore_ascii_case(k)))
}

pub(crate) fn is_excluded_workspace_path(rel: &str) -> bool {
    let lower = rel.to_ascii_lowercase();
    if lower == "memory-bank/local" || lower.starts_with("memory-bank/local/") {
        return true;
    }
    if is_minified_js(rel) {
        return true;
    }
    rel.split('/')
        .any(|comp| !comp.is_empty() && IGNORED_CODE_DIRS.contains(&comp))
}

pub(crate) fn is_scannable_code_path(path: &str) -> bool {
    !is_excluded_workspace_path(path) && has_code_extension(path)
}

fn test_folder(part: &str) -> bool {
    if TEST_FOLDERS.contains(&part) {
        return true;
    }
    ["tests", "test"].iter().any(|end| {
        let Some(at) = part.len().checked_sub(end.len()) else {
            return false;
        };
        if !part.is_char_boundary(at) || !part[at..].eq_ignore_ascii_case(end) {
            return false;
        }
        let before = &part[..at];
        before.is_empty()
            || before.ends_with(['.', '-', '_'])
            || (part[at..].starts_with('T')
                && before.ends_with(|letter: char| {
                    letter.is_ascii_lowercase() || letter.is_ascii_digit()
                }))
    })
}

/// Whether a file is a test by where it lies or how it is named across supported languages.
pub(crate) fn is_test_path(path: &str) -> bool {
    let (folders, name) = path.rsplit_once('/').unwrap_or(("", path));
    if folders.split('/').any(test_folder) {
        return true;
    }
    let mut dotted = name.split('.');
    let stem = dotted.next().unwrap_or("");
    let rest = dotted.collect::<Vec<_>>();
    let marked = rest.len() > 1
        && rest[..rest.len() - 1].iter().any(|part| {
            ["test", "tests", "spec", "specs", "cy", "e2e"].contains(part)
                || part.ends_with("-spec")
                || part.ends_with("-test")
        });
    marked
        || ["test", "tests", "conftest", "fixture", "fixtures"].contains(&stem)
        || stem.starts_with("test_")
        || ["_test", "_tests", "_spec", "_fixture", "_fixtures"]
            .iter()
            .any(|end| stem.ends_with(end))
        || ["TestCase", "Tests", "Test", "Fixtures", "Fixture"]
            .iter()
            .any(|end| {
                stem.strip_suffix(end).is_some_and(|before| {
                    before.ends_with(|letter: char| {
                        letter.is_ascii_lowercase() || letter.is_ascii_digit()
                    })
                })
            })
}

pub(crate) fn is_layer_code_path(path: &str) -> bool {
    !is_test_path(path) && has_code_extension(path)
}
