//! Source-derived Python 3.14 Path.glob/re.IGNORECASE component oracles.
//! These explicit modes exercise Windows matching on every host; filesystem
//! assertions below use the native host and do not claim Windows execution.
use super::path_matches;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static SERIAL: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "infra-path-glob-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(fs::canonicalize(root).unwrap())
    }
    fn directory(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn write(root: &Path, name: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b"ordinary path fixture\n").unwrap();
}

#[test]
fn selected_python_ignorecase_preserves_ranges_and_unicode_equivalences() {
    // Python 3.14 Lib/re/_compiler.py uses simple Unicode lower plus
    // Lib/re/_casefix.py; it does not lower strings or glob range endpoints.
    // Literal/class/negative I-quartet rows plus Greek and supplementary cases
    // intentionally distinguish Python IGNORECASE from Rust's flag alone.
    for (pattern, value, ignore_case, expected) in [
        ("I", "I", true, true),
        ("[I]", "I", true, true),
        ("[!I]", "I", true, false),
        ("I", "i", true, true),
        ("[I]", "i", true, true),
        ("[!I]", "i", true, false),
        ("I", "İ", true, true),
        ("[I]", "İ", true, true),
        ("[!I]", "İ", true, false),
        ("I", "ı", true, true),
        ("[I]", "ı", true, true),
        ("[!I]", "ı", true, false),
        ("i", "I", true, true),
        ("[i]", "I", true, true),
        ("[!i]", "I", true, false),
        ("i", "i", true, true),
        ("[i]", "i", true, true),
        ("[!i]", "i", true, false),
        ("i", "İ", true, true),
        ("[i]", "İ", true, true),
        ("[!i]", "İ", true, false),
        ("i", "ı", true, true),
        ("[i]", "ı", true, true),
        ("[!i]", "ı", true, false),
        ("İ", "I", true, true),
        ("[İ]", "I", true, true),
        ("[!İ]", "I", true, false),
        ("İ", "i", true, true),
        ("[İ]", "i", true, true),
        ("[!İ]", "i", true, false),
        ("İ", "İ", true, true),
        ("[İ]", "İ", true, true),
        ("[!İ]", "İ", true, false),
        ("İ", "ı", true, true),
        ("[İ]", "ı", true, true),
        ("[!İ]", "ı", true, false),
        ("ı", "I", true, true),
        ("[ı]", "I", true, true),
        ("[!ı]", "I", true, false),
        ("ı", "i", true, true),
        ("[ı]", "i", true, true),
        ("[!ı]", "i", true, false),
        ("ı", "İ", true, true),
        ("[ı]", "İ", true, true),
        ("[!ı]", "İ", true, false),
        ("ı", "ı", true, true),
        ("[ı]", "ı", true, true),
        ("[!ı]", "ı", true, false),
        ("[H-J]", "ı", true, true),
        ("[h-j]", "İ", true, true),
        ("[Z-a]", "a", true, true),
        ("[Z-a]", "A", true, true),
        ("[Z-a]", "z", true, true),
        ("[Z-a]", "I", true, false),
        ("[Z-a]", "İ", true, false),
        ("[Z-a]", "_", true, true),
        ("[İ-ı]", "I", true, true),
        ("[İ-ı]", "j", true, false),
        ("[!İ-ı]", "ı", true, false),
        ("[!İ-ı]", "j", true, true),
        ("[^I]", "^", true, true),
        ("[^I]", "ı", true, true),
        ("[!^I]", "^", true, false),
        ("[!^I]", "İ", true, false),
        ("[z-a]", "z", true, false),
        ("[!z-a]", "\n", true, true),
        ("[A-Z]", "ſ", true, true),
        ("[A-Z]", "K", true, true),
        ("Σ", "ς", true, true),
        ("Σ", "σ", true, true),
        ("[!Σ]", "ς", true, false),
        ("ϑ", "ϴ", true, true),
        ("ϑ", "Θ", true, true),
        ("[β-β]", "ϐ", true, true),
        ("µ", "μ", true, true),
        ("Ι", "ͅ", true, true),
        ("Ι", "ι", true, true),
        ("[!Ι]", "ͅ", true, false),
        ("ΐ", "ΐ", true, true),
        ("ΰ", "ΰ", true, true),
        ("𐐀", "𐐨", true, true),
        ("[𐐀-𐐁]", "𐐩", true, true),
        ("[!𐐀-𐐁]", "𐐨", true, false),
        ("[!𐐀-𐐁]", "𐐪", true, true),
        ("?", "💡", true, true),
        ("ß", "ss", true, false),
        ("İ", "i̇", true, false),
        ("ﬅ", "ﬆ", true, true),
        ("ﬅ", "st", true, false),
        ("[Z-a]", "a", false, true),
        ("[Z-a]", "A", false, false),
        ("I", "i", false, false),
        ("[I]", "ı", false, false),
        ("[!I]", "ı", false, true),
        ("Σ", "ς", false, false),
        ("𐐀", "𐐨", false, false),
        ("?", "💡", false, true),
    ] {
        assert_eq!(
            crate::common::glob_match_path(pattern, value, ignore_case).unwrap(),
            expected,
            "{pattern:?}/{value:?}, ignore_case={ignore_case}"
        );
    }
}

#[test]
fn path_matches_uses_host_case_rules_after_glob_translation() {
    let temp = Temp::new();
    for (index, pattern, name, expected) in [
        (0, "*.PHP", "Controller.php", cfg!(windows)),
        (1, "[Z-a].txt", "a.txt", true),
        (2, "[i].txt", "İ.txt", cfg!(windows)),
        (3, "[!i].txt", "ı.txt", !cfg!(windows)),
        (4, "[A-Z].txt", "ſ.txt", cfg!(windows)),
        (5, "[𐐀-𐐁].txt", "𐐨.txt", cfg!(windows)),
        (6, "[Σ].txt", "ς.txt", cfg!(windows)),
        (7, "*", ".hidden", true),
        (8, "**/*.PHP", "nested/Controller.php", cfg!(windows)),
    ] {
        // Separate roots avoid case-equivalent filenames colliding on Windows.
        let root = temp.directory(&format!("case-{index}"));
        write(&root, name);
        let paths = path_matches(&root, pattern).unwrap();
        assert_eq!(
            paths,
            if expected {
                vec![root.join(name)]
            } else {
                vec![]
            },
            "{pattern:?}/{name:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn path_glob_preserves_confined_link_refusal() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    let target = temp.directory("target");
    let outside = temp.directory("outside");
    write(&outside, "secret.php");
    symlink(outside.join("secret.php"), target.join("linked.php")).unwrap();
    symlink(&outside, target.join("linked-directory")).unwrap();
    assert!(path_matches(&target, "*.php").unwrap().is_empty());
    assert!(path_matches(&target, "**/*.php").unwrap().is_empty());
}
