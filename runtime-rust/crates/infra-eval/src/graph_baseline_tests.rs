//! Pure graph-baseline source admission and build-plan contracts.
use super::{baseline_build_spec, baseline_paths, native_baseline};
use crate::artifact;
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "infra-graph-baseline-{}",
            uuid::Uuid::new_v4().simple()
        ));
        artifact::private_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn argument<'a>(args: &'a [String], flag: &str) -> &'a str {
    let at = args.iter().position(|arg| arg == flag).unwrap();
    args.get(at + 1).unwrap()
}

#[test]
fn graph_baseline_paths_and_build_argv_use_the_native_engine() {
    let root = Root::new();
    let target = infra_tooling::artifacts::native_target().unwrap();
    let engine = if cfg!(windows) {
        "infra-engine.exe"
    } else {
        "infra-engine"
    };
    let paths = baseline_paths(&root.0).unwrap();
    assert_eq!(
        paths,
        [
            root.0
                .join("runtime-rust/target")
                .join(&target)
                .join("release")
                .join(engine),
            root.0.join("runtime-rust/target/release").join(engine),
            root.0.join("runtime-rust/bin").join(engine),
            root.0.join("bin").join(engine),
        ]
    );
    let spec = baseline_build_spec(&root.0).unwrap();
    assert_eq!(
        spec.argv[..4]
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["cargo", "build", "--locked", "--release"]
    );
    assert_eq!(argument(&spec.argv, "--bin"), "infra-engine");
    assert_eq!(argument(&spec.argv, "--target"), target);
    assert_eq!(
        Path::new(argument(&spec.argv, "--manifest-path")),
        root.0.join("runtime-rust/Cargo.toml")
    );
    assert_eq!(
        Path::new(argument(&spec.argv, "--target-dir")),
        root.0.join("runtime-rust/target")
    );
    assert_eq!(spec.cwd, root.0);
    if let Ok(path) = std::env::var("PATH") {
        assert_eq!(spec.env.get("PATH"), Some(&path));
    }
    if let Ok(home) = std::env::var("HOME") {
        assert_eq!(spec.env.get("HOME"), Some(&home));
    }
}

#[test]
fn graph_baseline_missing_native_source_is_explicit_even_with_override() {
    let root = Root::new();
    for build in [false, true] {
        let error = native_baseline(
            &root.0,
            Some("an-explicit-artifact-does-not-invent-native-source"),
            build,
        )
        .unwrap_err();
        assert_eq!(
            error.message,
            "native-baseline-unavailable: the committed reference contains no native runtime; choose a native commit"
        );
    }
    assert!(!root.0.join("runtime-rust/target").exists());
}
