//! Scratch creation must preserve existing shared-parent authority.
use super::Scratch;
use infra_core::bank::publication::PublicationFiles;
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "infra-eval-scratch-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn namespace(path: &Path) -> (u64, u64, u32) {
    PublicationFiles::new(path).unwrap().root_identity()
}
#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

#[test]
fn scratch_preserves_existing_parent_namespace_and_shared_permissions() {
    let root = Directory::new();
    let shared = root.0.join("shared");
    fs::create_dir(&shared).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o1777)).unwrap();
    }
    let before = namespace(&shared);
    let scratch = Scratch::new(&shared, "capture").unwrap();
    assert_eq!(scratch.0.parent(), Some(shared.as_path()));
    assert_eq!(namespace(&shared), before);
    #[cfg(unix)]
    {
        assert_eq!(mode(&shared), 0o1777);
        assert_eq!(mode(&scratch.0), 0o700);
    }
    let path = scratch.0.clone();
    drop(scratch);
    assert!(!path.exists());
    assert_eq!(namespace(&shared), before);
}

#[test]
fn scratch_creates_missing_parents_privately_without_tightening_existing_ones() {
    let root = Directory::new();
    let existing = root.0.join("existing");
    fs::create_dir(&existing).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&existing, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let before = namespace(&existing);
    let parent = existing.join("new/nested");
    let scratch = Scratch::new(&parent, "capture").unwrap();
    assert_eq!(namespace(&existing), before);
    assert_eq!(scratch.0.parent(), Some(parent.as_path()));
    #[cfg(unix)]
    {
        assert_eq!(mode(&existing), 0o755);
        assert_eq!(mode(&existing.join("new")), 0o700);
        assert_eq!(mode(&parent), 0o700);
        assert_eq!(mode(&scratch.0), 0o700);
    }
}

#[test]
fn scratch_refuses_a_collision_without_adopting_or_changing_its_directory() {
    let root = Directory::new();
    let occupied = root.0.join("occupied");
    fs::create_dir(&occupied).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&occupied, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(occupied.join("sentinel"), b"unrelated existing bytes").unwrap();
    let before = namespace(&occupied);
    assert!(Scratch::create(&root.0, "occupied").is_err());
    assert_eq!(namespace(&occupied), before);
    assert_eq!(
        fs::read(occupied.join("sentinel")).unwrap(),
        b"unrelated existing bytes"
    );
}

#[test]
fn scratch_refuses_file_parents_and_invalid_leaf_names_before_creation() {
    let root = Directory::new();
    let parent = root.0.join("not-a-directory");
    fs::write(&parent, b"existing file").unwrap();
    assert!(Scratch::new(&parent, "capture").is_err());
    assert_eq!(fs::read(&parent).unwrap(), b"existing file");
    let missing = root.0.join("must-remain-absent");
    for name in [
        "../escape",
        "nested/leaf",
        "/absolute",
        r"nested\leaf",
        "colon:leaf",
    ] {
        assert!(Scratch::create(&missing, name).is_err());
        assert!(
            !missing.exists(),
            "invalid name created its parent: {name:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn scratch_refuses_symlinked_parents_without_touching_the_destination() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = Directory::new();
    let outside = Directory::new();
    fs::set_permissions(&outside.0, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(outside.0.join("sentinel"), b"outside").unwrap();
    let before = namespace(&outside.0);
    let alias = root.0.join("alias");
    symlink(&outside.0, &alias).unwrap();
    assert!(Scratch::new(&alias, "capture").is_err());
    assert_eq!(namespace(&outside.0), before);
    assert_eq!(fs::read_dir(&outside.0).unwrap().count(), 1);
    assert_eq!(fs::read(outside.0.join("sentinel")).unwrap(), b"outside");
}
