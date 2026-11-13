//! Filesystem-only namespace contracts; no Git, host, provider or process runs.
use super::{ImageFile, copy_image};
#[cfg(unix)]
use super::{ImageKind, candidate_digest, image};
#[cfg(unix)]
use crate::{oracle, outcome};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
#[cfg(unix)]
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("infra-image-names-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        super::chmod(&path, 0o700).unwrap();
        Self(path)
    }
    fn directory(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir(&path).unwrap();
        path
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn write(root: &Path, name: impl AsRef<Path>, bytes: &[u8]) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
fn tree(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, at: &Path, rows: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for entry in fs::read_dir(at).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            assert!(!metadata.is_symlink());
            let name = path.strip_prefix(root).unwrap().to_path_buf();
            if metadata.is_dir() {
                rows.insert(name, None);
                visit(root, &path, rows);
            } else {
                assert!(metadata.is_file());
                rows.insert(name, Some(fs::read(&path).unwrap()));
            }
        }
    }
    let mut rows = BTreeMap::new();
    visit(root, root, &mut rows);
    rows
}
#[cfg(unix)]
type MetadataTree = (u32, BTreeMap<PathBuf, (Option<Vec<u8>>, u32)>);
#[cfg(unix)]
fn tree_with_modes(root: &Path) -> MetadataTree {
    let rows = tree(root)
        .into_iter()
        .map(|(name, bytes)| {
            let mode = super::mode(&root.join(&name)).unwrap();
            (name, (bytes, mode))
        })
        .collect();
    (super::mode(root).unwrap(), rows)
}
#[cfg(unix)]
#[test]
fn public_snapshots_refuse_native_backslash_after_private_exclusions() {
    let temp = Root::new();
    let source = temp.directory("public");
    write(&source, "документы/é.md", b"public content\n");
    write(&source, r".git/ignored\file.md", b"private\n");
    write(
        &source,
        OsString::from_vec(b".env.private\xff".to_vec()),
        b"private\n",
    );
    for kind in [ImageKind::Public, ImageKind::NativePublic] {
        let rows = image(&source, kind, None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "документы/é.md");
    }
    write(&source, r"a\b.md", b"public namespace witness\n");
    let before = tree(&source);
    for kind in [ImageKind::Public, ImageKind::NativePublic] {
        assert!(image(&source, kind, None).is_err());
    }
    assert_eq!(tree(&source), before);
}
#[cfg(unix)]
#[test]
fn native_candidate_physical_and_oracle_names_do_not_alias_a_directory_separator() {
    let temp = Root::new();
    let literal = temp.directory("literal");
    let nested = temp.directory("nested");
    // Both oracle directory sets contain "a"; only the native file names differ.
    fs::create_dir(literal.join("a")).unwrap();
    write(&literal, r"a\b.md", b"same content\n");
    write(&nested, "a/b.md", b"same content\n");
    let before = tree(&literal);
    let before_modes = tree_with_modes(&literal);
    for kind in [ImageKind::Physical, ImageKind::Candidate, ImageKind::Oracle] {
        let rows = image(&literal, kind, None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, r"a\b.md");
        let other = image(&nested, kind, None).unwrap();
        assert_eq!(other[0].path, "a/b.md");
        assert_ne!(
            super::image_digest(&rows, false).unwrap(),
            super::image_digest(&other, false).unwrap()
        );
    }
    assert_ne!(
        candidate_digest(&literal).unwrap(),
        candidate_digest(&nested).unwrap()
    );
    assert_ne!(
        oracle::oracle_digest(&literal).unwrap(),
        oracle::oracle_digest(&nested).unwrap()
    );
    let copied = temp.0.join("physical-copy");
    outcome::copy_corpus(&literal, &copied).unwrap();
    assert_eq!(fs::read(copied.join(r"a\b.md")).unwrap(), b"same content\n");
    assert!(!copied.join("a/b.md").exists());
    assert_eq!(
        tree_with_modes(&copied),
        before_modes,
        "native copy must preserve the complete directory tree and modes"
    );
    let portable = temp.0.join("portable-refusal");
    assert!(
        copy_image(
            &image(&literal, ImageKind::Candidate, None).unwrap(),
            &portable
        )
        .is_err()
    );
    assert!(
        !portable.exists(),
        "portable copy validation must remain strict"
    );
    assert_eq!(tree(&literal), before);
}
#[cfg(unix)]
#[test]
fn serialized_snapshots_refuse_invalid_utf8_but_keep_a_real_replacement_character() {
    let temp = Root::new();
    let invalid = temp.directory("invalid");
    let valid = temp.directory("valid");
    write(
        &invalid,
        OsString::from_vec(b"note\xff.md".to_vec()),
        b"same content\n",
    );
    write(&valid, "note\u{fffd}.md", b"same content\n");
    let before = tree(&invalid);
    for kind in [
        ImageKind::Public,
        ImageKind::NativePublic,
        ImageKind::Physical,
        ImageKind::Candidate,
        ImageKind::Oracle,
    ] {
        assert!(image(&invalid, kind, None).is_err());
        let rows = image(&valid, kind, None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "note\u{fffd}.md");
    }
    assert_eq!(tree(&invalid), before);
}
#[cfg(unix)]
#[test]
fn oracle_empty_directory_spelling_is_part_of_the_sealed_identity() {
    let temp = Root::new();
    let literal = temp.directory("literal-directories");
    let nested = temp.directory("nested-directories");
    fs::create_dir(literal.join("a")).unwrap();
    fs::create_dir(literal.join(r"a\b")).unwrap();
    fs::create_dir_all(nested.join("a/b")).unwrap();
    let before = tree(&literal);
    assert_ne!(
        oracle::oracle_digest(&literal).unwrap(),
        oracle::oracle_digest(&nested).unwrap()
    );
    assert_eq!(tree(&literal), before);
}
#[test]
fn portable_copy_keeps_traversal_controls_and_utf8_names() {
    let temp = Root::new();
    for (index, name) in [
        "../escape",
        "/escape",
        "a/../escape",
        "a//b",
        "a\0b",
        r"a\b",
        "a:b",
    ]
    .iter()
    .enumerate()
    {
        let target = temp.0.join(format!("refused-{index}"));
        let rows = vec![ImageFile {
            path: (*name).into(),
            bytes: b"not written".to_vec(),
            mode: 0o600,
        }];
        assert!(copy_image(&rows, &target).is_err());
        assert!(!target.exists());
    }
    let target = temp.0.join("valid-copy");
    copy_image(
        &[ImageFile {
            path: "документы/é.md".into(),
            bytes: b"preserved\n".to_vec(),
            mode: 0o600,
        }],
        &target,
    )
    .unwrap();
    assert_eq!(
        fs::read(target.join("документы/é.md")).unwrap(),
        b"preserved\n"
    );
}

#[cfg(unix)]
#[test]
fn physical_copy_preserves_empty_directories_root_and_directory_modes() {
    let temp = Root::new();
    let source = temp.directory("ordinary-source");
    fs::create_dir_all(source.join("empty/leaf")).unwrap();
    write(&source, "nonempty/file", b"ordinary physical source\n");
    for (path, mode) in [
        (source.clone(), 0o750),
        (source.join("empty"), 0o710),
        (source.join("empty/leaf"), 0o700),
        (source.join("nonempty"), 0o755),
        (source.join("nonempty/file"), 0o640),
    ] {
        super::chmod(&path, mode).unwrap();
    }
    let before = tree_with_modes(&source);
    let source_digest = oracle::oracle_digest(&source).unwrap();
    let copied = temp.0.join("ordinary-copy");
    outcome::copy_corpus(&source, &copied).unwrap();
    let copied_tree = tree_with_modes(&copied);
    let copied_digest = oracle::oracle_digest(&copied).unwrap();
    assert_eq!(
        tree_with_modes(&source),
        before,
        "copy must leave its source unchanged"
    );
    assert_eq!(
        copied_tree, before,
        "native physical copy must retain empty directories and every ordinary mode"
    );
    assert_eq!(
        copied_digest, source_digest,
        "the copy must retain the sealed oracle identity"
    );
}

#[cfg(unix)]
#[test]
fn private_output_admission_protects_only_selected_directory_and_new_ancestors() {
    let temp = Root::new();
    let public = temp.directory("existing-public");
    super::chmod(&public, 0o775).unwrap();
    let selected = public.join("new").join("owned-output");
    super::private_dir(&selected).unwrap();
    assert_eq!(super::mode(&public).unwrap(), 0o775);
    assert_eq!(super::mode(&public.join("new")).unwrap(), 0o700);
    assert_eq!(super::mode(&selected).unwrap(), 0o700);
    super::chmod(&selected, 0o755).unwrap();
    super::private_dir(&selected).unwrap();
    assert_eq!(super::mode(&selected).unwrap(), 0o700);
    assert_eq!(super::mode(&public).unwrap(), 0o775);
}

#[cfg(unix)]
#[test]
fn native_copy_creates_literal_private_target_without_tightening_existing_ancestors() {
    let temp = Root::new();
    let source = temp.directory("native-target-source");
    write(&source, r"é\file:literal.md", b"native copy witness\n");
    let source_before = tree_with_modes(&source);
    let rows = image(&source, ImageKind::Candidate, None).unwrap();
    let public = temp.directory("public-copy-parent");
    super::chmod(&public, 0o775).unwrap();
    let parent = public.join(r"new\parent:literal");
    let target = parent.join(r"copy\leaf:literal");
    super::copy_native_image(&rows, &target).unwrap();
    assert_eq!(super::mode(&public).unwrap(), 0o775);
    assert_eq!(super::mode(&parent).unwrap(), 0o700);
    assert_eq!(super::mode(&target).unwrap(), 0o700);
    assert_eq!(tree_with_modes(&source), source_before);
    assert_eq!(
        fs::read(target.join(r"é\file:literal.md")).unwrap(),
        b"native copy witness\n"
    );
    assert!(!public.join("new").exists());
    assert!(!parent.join("copy").exists());
    assert!(!target.join("é").exists());
}

#[test]
fn native_copy_collision_preserves_existing_target_and_its_contents() {
    let temp = Root::new();
    let target = temp.directory("preexisting-native-target");
    write(&target, "sentinel", b"unrelated existing output\n");
    let before = tree(&target);
    let mode = super::mode(&target).unwrap();
    let rows = [ImageFile {
        path: "new-file.md".into(),
        bytes: b"must not adopt target".to_vec(),
        mode: 0o600,
    }];
    assert!(super::copy_native_image(&rows, &target).is_err());
    assert_eq!(tree(&target), before);
    assert_eq!(super::mode(&target).unwrap(), mode);
}

#[cfg(unix)]
#[test]
fn native_copy_refuses_non_utf8_target_before_creating_its_missing_parent() {
    let temp = Root::new();
    let parent = temp.0.join("must-remain-missing");
    let target = parent.join(OsString::from_vec(b"target-\xff".to_vec()));
    let before = tree(&temp.0);
    assert!(super::copy_native_image(&[], &target).is_err());
    assert_eq!(tree(&temp.0), before);
    assert!(!parent.exists());
}

#[cfg(unix)]
#[test]
fn native_tree_metadata_and_error_cleanup_refuse_foreign_root_replacements() {
    for return_error in [false, true] {
        let temp = Root::new();
        let source = temp.directory("source-for-replacement");
        write(&source, "nonempty/file", b"retained owned bytes\n");
        fs::create_dir_all(source.join("empty/leaf")).unwrap();
        super::chmod(&source, 0o750).unwrap();
        super::chmod(&source.join("empty"), 0o710).unwrap();
        let source_before = tree_with_modes(&source);
        let target = temp.0.join("copy-target");
        let retained = temp.0.join("owned-copy-retained");
        let foreign_before = std::cell::RefCell::new(None);
        let error =
            super::copy_native_tree_using(&source, &target, ImageKind::Physical, None, |path| {
                fs::rename(path, &retained).unwrap();
                fs::create_dir(path).unwrap();
                super::chmod(path, 0o755).unwrap();
                write(path, "sentinel", b"foreign replacement must survive\n");
                *foreign_before.borrow_mut() = Some(tree_with_modes(path));
                if return_error {
                    Err(super::invalid("injected metadata error"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert!(
            error.kind == infra_core::ErrorKind::Conflict
                || error.kind == infra_core::ErrorKind::InvalidInput
        );
        assert_eq!(
            tree_with_modes(&target),
            foreign_before.into_inner().unwrap()
        );
        assert!(!target.join("empty").exists());
        assert_eq!(
            fs::read(retained.join("nonempty/file")).unwrap(),
            b"retained owned bytes\n"
        );
        assert_eq!(tree_with_modes(&source), source_before);
    }
}

#[cfg(unix)]
#[test]
fn native_copy_preserves_raw_empty_directory_names_and_modes_without_serializing_them() {
    let temp = Root::new();
    let source = temp.directory("raw-empty-source");
    let raw = OsString::from_vec(b"empty-\\literal:\xff".to_vec());
    let empty = source.join(&raw);
    fs::create_dir_all(empty.join("leaf")).unwrap();
    write(&source, "ordinary/file", b"ordinary source bytes\n");
    super::chmod(&source, 0o750).unwrap();
    super::chmod(&empty, 0o710).unwrap();
    let before = tree_with_modes(&source);
    let target = temp.0.join("raw-empty-copy");
    outcome::copy_corpus(&source, &target).unwrap();
    assert_eq!(tree_with_modes(&target), before);
    assert_eq!(tree_with_modes(&source), before);
    assert!(target.join(&raw).join("leaf").is_dir());
}
