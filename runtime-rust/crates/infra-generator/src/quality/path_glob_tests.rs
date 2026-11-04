//! Source-derived Python 3.14 Path.glob/re.IGNORECASE component oracles.
//! These explicit modes exercise Windows matching on every host; filesystem
//! assertions below use the native host and do not claim Windows execution.
use super::{PathMatcher, confined, path_matches, relative, segments, wildcard};
use infra_core::Result;
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

// Frozen exhaustive walker from before literal-prefix pruning. Matching still
// uses the independently-oracled component semantics; recursion never prunes.
fn exhaustive_reference(target: &Path, value: &str) -> Result<Vec<PathBuf>> {
    if !wildcard(value) {
        return Ok(confined(target, value)
            .filter(|p| p.exists())
            .into_iter()
            .collect());
    }
    fn walk(
        root: &Path,
        directory: &Path,
        matcher: &mut PathMatcher<'_>,
        output: &mut Vec<PathBuf>,
    ) -> Result<()> {
        let Ok(entries) = fs::read_dir(directory) else {
            return Ok(());
        };
        for entry in entries.flatten() {
            let candidate = entry.path();
            let name = relative(root, &candidate);
            if matcher.matches(&segments(&name))?
                && let Some(path) = confined(root, &name).filter(|p| p.exists())
            {
                output.push(path);
            }
            if entry
                .file_type()
                .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
            {
                walk(root, &candidate, matcher, output)?;
            }
        }
        Ok(())
    }
    let pattern = segments(value);
    let mut matcher = PathMatcher::new(&pattern, cfg!(windows));
    let mut output = Vec::new();
    walk(target, target, &mut matcher, &mut output)?;
    output.sort();
    output.dedup();
    Ok(output)
}

fn error_image(error: infra_core::RuntimeError) -> serde_json::Value {
    serde_json::json!({"kind":format!("{:?}",error.kind),"message":error.message,
        "code":error.code,"os_code":error.os_code})
}

fn assert_reference(target: &Path, pattern: &str) {
    match (
        path_matches(target, pattern),
        exhaustive_reference(target, pattern),
    ) {
        (Ok(actual), Ok(expected)) => assert_eq!(actual, expected, "{pattern:?}"),
        (Err(actual), Err(expected)) => {
            assert_eq!(error_image(actual), error_image(expected), "{pattern:?}")
        }
        (actual, expected) => panic!("Different outcomes for {pattern:?}: {actual:?}/{expected:?}"),
    }
}

#[test]
fn literal_prefix_pruning_matches_exhaustive_walk_across_adversarial_patterns() {
    let temp = Temp::new();
    let root = temp.directory("target");
    for name in [
        "src/Feature/Foo.php",
        "src/Feature/nested/Bar.php",
        "src/.hidden.php",
        "src/notes.txt",
        "src-extra/Other.php",
        "src[/Literal.php",
        "vendor/Package/Dependency.php",
        "unrelated/nested/deeper/File.php",
    ] {
        write(&root, name);
    }
    fs::create_dir(root.join("src/empty")).unwrap();
    for first in [
        "src", "src*", "**", "[s]rc", "missing", ".", "..", "Src", "src[",
    ] {
        for tail in [
            "**",
            "**/*.php",
            "*/Foo.php",
            "**/**/Foo.php",
            "**/*.missing",
        ] {
            assert_reference(&root, &format!("{first}/{tail}"));
        }
    }
    for pattern in [
        "src/Feature/**/*.php",
        "src/Missing/**/*.php",
        "src/Feature/**/nested/**",
        "src/Feature/[!F]*.php",
        "src/Feature/[unclosed",
        "./src//Feature/**/*.php",
        "src/../**",
        "src/Feature/Foo.php",
    ] {
        assert_reference(&root, pattern);
    }
    let empty_root = temp.directory("empty-target");
    for pattern in ["src/**/[a", "src/**", "**", "../**"] {
        assert_reference(&empty_root, pattern);
    }
    fs::create_dir(empty_root.join("src")).unwrap();
    assert_eq!(
        path_matches(&empty_root, "src/**").unwrap(),
        vec![empty_root.join("src")]
    );
    assert_reference(&empty_root, "src/**");
}

#[test]
fn literal_prefix_checks_preserve_case_rules_and_component_reachability() {
    for (prefix, directory, ignore_case, expected) in [
        ("I", "ı", true, true),
        ("I", "İ", true, true),
        ("I", "ı", false, false),
        ("Src", "src", true, true),
        ("Src", "src", false, false),
        ("Σ", "ς", true, true),
        ("Σ", "ς", false, false),
    ] {
        let pattern = segments(&format!("{prefix}/**/[unclosed"));
        let directory = segments(directory);
        let mut matcher = PathMatcher::new(&pattern, ignore_case);
        assert!(!matcher.matches(&directory).unwrap());
        let reached = matcher.compilations;
        assert_eq!(matcher.can_descend(&directory), expected);
        assert_eq!(matcher.compilations, reached, "reached a new expression");
        assert!(matcher.compiled[2].is_none());
    }
    for pattern in ["src*/**", "**/*.php", "[s]rc/**", "src?/**"] {
        let components = segments(pattern);
        let mut matcher = PathMatcher::new(&components, false);
        let directory = segments("unrelated/nested");
        matcher.matches(&directory).unwrap();
        let reached = matcher.compilations;
        assert!(matcher.can_descend(&directory), "{pattern}");
        assert_eq!(matcher.compilations, reached);
        assert_eq!(matcher.literal_prefix, 0);
    }
    let pattern = vec!["src".into(), "x".repeat(1_000_000), "[".into()];
    let directory = segments("src");
    for ignore_case in [false, true] {
        let mut matcher = PathMatcher::new(&pattern, ignore_case);
        assert!(!matcher.matches(&directory).unwrap());
        assert_eq!(matcher.compilations, 1);
        assert!(matcher.can_descend(&directory));
        assert_eq!(matcher.compilations, 1, "compiled an absent component");
        assert!(matcher.compiled[1].is_none());
        assert!(matcher.compiled[2].is_none());
    }
}

#[test]
fn pruning_never_recompiles_absent_or_rejected_cached_literals() {
    for ignore_case in [false, true] {
        let pattern = segments("src/Feature/**");
        let mut matcher = PathMatcher::new(&pattern, ignore_case);
        assert!(!matcher.matches(&segments("src/Other")).unwrap());
        let reached = matcher.compilations;
        // A case-sensitive compiler rejection is retained as None. Inject that
        // cache image directly instead of compiling a huge expression in tests.
        matcher.compiled[1] = None;
        assert!(!matcher.can_descend(&segments("src/Feature")));
        assert_eq!(matcher.compilations, reached);
        assert!(matcher.compiled[1].is_none());

        let mut absent = PathMatcher::new(&pattern, ignore_case);
        assert!(!absent.matches(&segments("src")).unwrap());
        assert!(absent.can_descend(&segments("src")));
        assert!(!absent.can_descend(&segments("src/Feature")));
        assert_eq!(absent.compilations, 1);
        assert!(absent.compiled[1].is_none());
    }
}

#[cfg(unix)]
#[test]
fn prefix_pruning_retains_exhaustive_lossy_filename_and_link_behavior() {
    use std::{
        ffi::OsString,
        os::unix::{ffi::OsStringExt, fs::symlink},
    };
    let temp = Temp::new();
    let root = temp.directory("target");
    let outside = temp.directory("outside");
    write(&root, "src/Feature/Foo.php");
    write(&outside, "Outside.php");
    symlink(&outside, root.join("linked-outside")).unwrap();
    symlink(root.join("src"), root.join("linked-inside")).unwrap();
    symlink(outside.join("Outside.php"), root.join("src/alias.php")).unwrap();
    symlink(outside.join("missing.php"), root.join("dangling.php")).unwrap();
    let non_utf8 = root.join(OsString::from_vec(b"src\xff".to_vec()));
    fs::create_dir(&non_utf8).unwrap();
    fs::write(non_utf8.join("OnlyPhysical.php"), b"non-UTF8 path fixture").unwrap();
    write(&root, "src\u{fffd}/OnlyUnicode.php");
    let backslash_name = root.join(r"src\Backslash");
    fs::create_dir(&backslash_name).unwrap();
    fs::write(
        backslash_name.join("OnlyPhysical.php"),
        b"backslash fixture",
    )
    .unwrap();
    write(&root, "src/Backslash/OnlyUnicode.php");
    for pattern in [
        "src/**/*.php",
        "src\u{fffd}/**",
        "src/Backslash/**",
        r"src\Backslash/**",
        "**/*.php",
        "linked-outside/**",
        "linked-inside/**",
        "src/alias*",
        "dangling*",
    ] {
        assert_reference(&root, pattern);
    }
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
        ("literal[.txt", "literal[.txt", false, true),
        ("literal[.txt", "literal[.txt", true, true),
        ("[]", "[]", false, true),
        ("[]", "[]", true, true),
        ("[!a]", "a", false, false),
        ("[!a]", "b", false, true),
    ] {
        assert_eq!(
            crate::common::glob_match_path(pattern, value, ignore_case).unwrap(),
            expected,
            "{pattern:?}/{value:?}, ignore_case={ignore_case}"
        );
        let components = vec![pattern.to_owned()];
        let mut cached = PathMatcher::new(&components, ignore_case);
        for _ in 0..3 {
            assert_eq!(
                cached.matches(&[value.to_owned()]).unwrap(),
                expected,
                "cached {pattern:?}/{value:?}, ignore_case={ignore_case}"
            );
        }
    }
}

#[test]
fn path_matches_keeps_complete_sorted_results_for_recursive_and_literal_components() {
    let temp = Temp::new();
    let root = temp.directory("target");
    for name in [
        "README.md",
        "literal[.txt",
        "src/Foo.php",
        "src/.hidden.php",
        "src/Feature/Bar.PHP",
        "vendor/Tool.php",
    ] {
        write(&root, name);
    }
    fs::create_dir(root.join("src/empty")).unwrap();
    let mut source_php = vec!["src/.hidden.php", "src/Foo.php"];
    if cfg!(windows) {
        source_php.push("src/Feature/Bar.PHP");
    }
    let mut all_php = source_php.clone();
    all_php.push("vendor/Tool.php");
    for (pattern, expected) in [
        ("src/**/*.php", source_php),
        ("**/*.php", all_php),
        ("src/Foo*.php", vec!["src/Foo.php"]),
        ("src/[F]oo.php", vec!["src/Foo.php"]),
        ("src/[!F]*.php", vec!["src/.hidden.php"]),
        ("src/**/**/Foo.php", vec!["src/Foo.php"]),
        ("src/**/empty", vec!["src/empty"]),
        ("literal[.txt", vec!["literal[.txt"]),
        ("missing/**/*.php", vec![]),
        ("src/**/*.missing", vec![]),
        (
            "src/**",
            vec![
                "src",
                "src/.hidden.php",
                "src/Foo.php",
                "src/Feature",
                "src/Feature/Bar.PHP",
                "src/empty",
            ],
        ),
        (
            "**",
            vec![
                "README.md",
                "literal[.txt",
                "src",
                "src/.hidden.php",
                "src/Foo.php",
                "src/Feature",
                "src/Feature/Bar.PHP",
                "src/empty",
                "vendor",
                "vendor/Tool.php",
            ],
        ),
    ] {
        let mut expected = expected
            .into_iter()
            .map(|name| root.join(name))
            .collect::<Vec<_>>();
        expected.sort();
        assert_eq!(path_matches(&root, pattern).unwrap(), expected, "{pattern}");
    }
}

#[test]
fn component_compilation_is_lazy_and_reused_across_paths() {
    for ignore_case in [false, true] {
        let pattern = segments("src/**/*.php");
        let mut matcher = PathMatcher::new(&pattern, ignore_case);
        assert_eq!(matcher.compilations, 0);
        assert!(!matcher.matches(&[]).unwrap());
        assert_eq!(matcher.compilations, 0);
        assert!(!matcher.matches(&segments("vendor/Other.php")).unwrap());
        assert_eq!(matcher.compilations, 1);
        assert!(matcher.matches(&segments("src/Foo.php")).unwrap());
        assert_eq!(matcher.compilations, 2);
        assert!(matcher.matches(&segments("src/Feature/Bar.php")).unwrap());
        assert!(!matcher.matches(&segments("src/Feature/notes.txt")).unwrap());
        assert_eq!(matcher.compilations, 2);
        assert!(matcher.compiled[1].is_none(), "** must remain structural");
    }
    let temp = Temp::new();
    assert!(path_matches(&temp.0, "src/**/[a").unwrap().is_empty());
    let empty = temp.directory("src");
    assert_eq!(path_matches(&temp.0, "src/**").unwrap(), vec![empty]);
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

    write(&target, "src/real.php");
    symlink(target.join("src/real.php"), target.join("src/alias.php")).unwrap();
    symlink(target.join("src"), target.join("linked-inside")).unwrap();
    symlink(outside.join("missing.php"), target.join("dangling.php")).unwrap();
    assert_eq!(
        path_matches(&target, "**/*.php").unwrap(),
        vec![target.join("src/real.php")]
    );
}
