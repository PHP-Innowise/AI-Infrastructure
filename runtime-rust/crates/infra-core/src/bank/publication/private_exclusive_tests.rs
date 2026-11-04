//! Exclusive private creation must never adopt a preexisting directory.
use super::PublicationFiles;
use std::{fs, path::PathBuf};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "infra-private-exclusive-{}",
            crate::domain::uuid4()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn private_creation_is_exclusive_and_retains_its_namespace() {
    let root = Directory::new();
    let mut owner = PublicationFiles::new(&root.0).unwrap();
    let created = owner.create_private_directory_exclusive("fresh").unwrap();
    assert_eq!(
        PublicationFiles::new(&root.0.join("fresh"))
            .unwrap()
            .root_identity(),
        created
    );
    owner.create_directory_exclusive("existing", 0o755).unwrap();
    fs::write(root.0.join("existing/sentinel"), b"existing bytes").unwrap();
    let before = PublicationFiles::new(&root.0.join("existing"))
        .unwrap()
        .root_identity();
    assert!(
        owner
            .create_private_directory_exclusive("existing")
            .is_err()
    );
    assert_eq!(
        PublicationFiles::new(&root.0.join("existing"))
            .unwrap()
            .root_identity(),
        before
    );
    assert_eq!(
        fs::read(root.0.join("existing/sentinel")).unwrap(),
        b"existing bytes"
    );
    owner.check().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(root.0.join("fresh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
}

#[cfg(windows)]
mod windows_creation {
    use super::super::{native, private_mkdir};
    use super::*;
    use std::{ffi::c_void, os::windows::io::AsRawHandle, ptr};
    #[repr(C)]
    struct Acl {
        _revision: u8,
        _reserved: u8,
        _bytes: u16,
        count: u16,
        _reserved2: u16,
    }
    #[repr(C)]
    struct AceHeader {
        kind: u8,
        flags: u8,
        bytes: u16,
    }
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn GetSecurityInfo(
            handle: *mut c_void,
            kind: u32,
            information: u32,
            owner: *mut *mut c_void,
            group: *mut *mut c_void,
            dacl: *mut *mut c_void,
            sacl: *mut *mut c_void,
            descriptor: *mut *mut c_void,
        ) -> u32;
        fn GetSecurityDescriptorControl(
            descriptor: *mut c_void,
            control: *mut u16,
            revision: *mut u32,
        ) -> i32;
        fn GetAce(acl: *mut c_void, index: u32, ace: *mut *mut c_void) -> i32;
        fn EqualSid(left: *mut c_void, right: *mut c_void) -> i32;
        fn CreateWellKnownSid(
            kind: u32,
            domain: *mut c_void,
            sid: *mut c_void,
            bytes: *mut u32,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }
    struct Descriptor(*mut c_void);
    impl Drop for Descriptor {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { LocalFree(self.0) };
            }
        }
    }
    fn assert_private_directory(created: &std::fs::File) {
        let mut owner = ptr::null_mut();
        let mut dacl = ptr::null_mut();
        let mut descriptor = Descriptor(ptr::null_mut());
        assert_eq!(
            unsafe {
                GetSecurityInfo(
                    created.as_raw_handle().cast(),
                    1,
                    1 | 4,
                    &mut owner,
                    ptr::null_mut(),
                    &mut dacl,
                    ptr::null_mut(),
                    &mut descriptor.0,
                )
            },
            0
        );
        assert!(!owner.is_null() && !dacl.is_null() && !descriptor.0.is_null());
        let mut control = 0u16;
        let mut revision = 0u32;
        assert_ne!(
            unsafe { GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision) },
            0
        );
        assert_ne!(control & 0x1000, 0, "DACL must be protected at creation");
        let acl = unsafe { &*dacl.cast::<Acl>() };
        assert_eq!(
            acl.count, 2,
            "only the current owner and SYSTEM are admitted"
        );
        let mut system = [0u32; 17];
        let mut system_bytes = std::mem::size_of_val(&system) as u32;
        assert_ne!(
            unsafe {
                CreateWellKnownSid(
                    22,
                    ptr::null_mut(),
                    system.as_mut_ptr().cast(),
                    &mut system_bytes,
                )
            },
            0
        );
        let mut owner_seen = false;
        let mut system_seen = false;
        for index in 0..u32::from(acl.count) {
            let mut ace = ptr::null_mut();
            assert_ne!(unsafe { GetAce(dacl, index, &mut ace) }, 0);
            let header = unsafe { &*ace.cast::<AceHeader>() };
            assert_eq!(header.kind, 0, "expected an explicit allow ACE");
            assert_eq!(header.flags & 0x10, 0, "inherited ACE is forbidden");
            assert!(header.bytes >= 12);
            let sid = unsafe { ace.cast::<u8>().add(8).cast::<c_void>() };
            let is_owner = unsafe { EqualSid(sid, owner) } != 0;
            let is_system = unsafe { EqualSid(sid, system.as_mut_ptr().cast()) } != 0;
            assert!(
                is_owner || is_system,
                "unexpected authority in private DACL"
            );
            owner_seen |= is_owner;
            system_seen |= is_system;
        }
        assert!(owner_seen && system_seen);
    }

    #[test]
    fn private_security_exists_on_the_initial_creation_handle() {
        let root = Directory::new();
        let parent = native::root(&root.0).unwrap();
        // Inspect the native primitive directly: no later secure/open helper
        // has had an opportunity to repair an inherited permissive DACL.
        let created = private_mkdir(&parent, "created-private").unwrap();
        assert_private_directory(&created);
    }

    struct SecurityImage {
        _descriptor: Descriptor,
        owner: *mut c_void,
        control: u16,
        acl: Vec<u8>,
    }
    impl SecurityImage {
        fn capture(file: &std::fs::File) -> Self {
            let mut owner = ptr::null_mut();
            let mut dacl = ptr::null_mut();
            let mut descriptor = Descriptor(ptr::null_mut());
            assert_eq!(
                unsafe {
                    GetSecurityInfo(
                        file.as_raw_handle().cast(),
                        1,
                        1 | 4,
                        &mut owner,
                        ptr::null_mut(),
                        &mut dacl,
                        ptr::null_mut(),
                        &mut descriptor.0,
                    )
                },
                0
            );
            assert!(!owner.is_null() && !dacl.is_null() && !descriptor.0.is_null());
            let mut control = 0u16;
            let mut revision = 0u32;
            assert_ne!(
                unsafe { GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision) },
                0
            );
            let size = usize::from(unsafe { &*dacl.cast::<Acl>() }._bytes);
            assert!(size >= std::mem::size_of::<Acl>());
            let acl = unsafe { std::slice::from_raw_parts(dacl.cast::<u8>(), size) }.to_vec();
            Self {
                _descriptor: descriptor,
                owner,
                control,
                acl,
            }
        }
        fn assert_unchanged(&self, file: &std::fs::File) {
            let after = Self::capture(file);
            assert_eq!(
                after.control, self.control,
                "existing security control changed"
            );
            assert_eq!(after.acl, self.acl, "existing directory DACL changed");
            assert_ne!(
                unsafe { EqualSid(after.owner, self.owner) },
                0,
                "existing owner changed"
            );
        }
    }

    #[test]
    fn ensure_private_directory_protects_each_new_component_and_preserves_existing_security() {
        let root = Directory::new();
        let parent = native::root(&root.0).unwrap();
        let parent_security = SecurityImage::capture(&parent);
        let parent_readonly = parent.metadata().unwrap().permissions().readonly();
        // Existing true/false admissions must never tighten an unrelated parent.
        crate::fs::ensure_directory(&root.0, true).unwrap();
        parent_security.assert_unchanged(&parent);
        let first = root.0.join("new-private");
        let leaf = first.join("nested-private");
        crate::fs::ensure_directory(&leaf, true).unwrap();
        assert_private_directory(&native::root(&first).unwrap());
        assert_private_directory(&native::root(&leaf).unwrap());
        parent_security.assert_unchanged(&parent);
        assert_eq!(
            parent.metadata().unwrap().permissions().readonly(),
            parent_readonly
        );

        // false keeps its ordinary creation policy, and true must not repair an
        // already-existing directory's ACL or readonly attribute afterwards.
        let ordinary = root.0.join("ordinary");
        crate::fs::ensure_directory(&ordinary, false).unwrap();
        let ordinary = native::root(&ordinary).unwrap();
        let ordinary_security = SecurityImage::capture(&ordinary);
        let ordinary_readonly = ordinary.metadata().unwrap().permissions().readonly();
        crate::fs::ensure_directory(&root.0.join("ordinary"), true).unwrap();
        ordinary_security.assert_unchanged(&ordinary);
        assert_eq!(
            ordinary.metadata().unwrap().permissions().readonly(),
            ordinary_readonly
        );
        parent_security.assert_unchanged(&parent);
    }
}

#[test]
fn ensure_private_empty_path_is_refused_instead_of_admitting_the_current_directory() {
    assert!(crate::fs::ensure_directory(std::path::Path::new(""), true).is_err());
    assert!(crate::fs::ensure_directory(std::path::Path::new(""), false).is_err());
}

#[test]
fn native_private_leaf_is_exclusive_and_preserves_collisions() {
    let root = Directory::new();
    let mut owner = PublicationFiles::new(&root.0).unwrap();
    let created = owner
        .create_private_native_directory_exclusive(std::ffi::OsStr::new("é-目录"))
        .unwrap();
    assert_eq!(
        PublicationFiles::new(&root.0.join("é-目录"))
            .unwrap()
            .root_identity(),
        created
    );
    fs::create_dir(root.0.join("collision")).unwrap();
    fs::write(
        root.0.join("collision/sentinel"),
        b"existing directory bytes",
    )
    .unwrap();
    fs::write(root.0.join("file-collision"), b"existing file bytes").unwrap();
    let before = PublicationFiles::new(&root.0.join("collision"))
        .unwrap()
        .root_identity();
    for name in ["collision", "file-collision"] {
        assert!(
            owner
                .create_private_native_directory_exclusive(std::ffi::OsStr::new(name))
                .is_err()
        );
    }
    assert_eq!(
        PublicationFiles::new(&root.0.join("collision"))
            .unwrap()
            .root_identity(),
        before
    );
    assert_eq!(
        fs::read(root.0.join("collision/sentinel")).unwrap(),
        b"existing directory bytes"
    );
    assert_eq!(
        fs::read(root.0.join("file-collision")).unwrap(),
        b"existing file bytes"
    );
    owner.check().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(root.0.join("é-目录"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
}

#[test]
fn native_private_leaf_refuses_components_before_filesystem_effects() {
    let root = Directory::new();
    let mut owner = PublicationFiles::new(&root.0).unwrap();
    let mut names = vec!["", ".", "..", "/", "../escape", "a/b", "a//b", "a/", "a\0b"];
    if !cfg!(unix) {
        names.extend([r"a\b", "a:b", r"C:\escape"]);
    }
    for name in names {
        let error = owner
            .create_private_native_directory_exclusive(std::ffi::OsStr::new(name))
            .unwrap_err();
        assert_eq!(error.kind, crate::ErrorKind::InvalidInput);
        assert_eq!(fs::read_dir(&root.0).unwrap().count(), 0, "{name:?}");
        owner.check().unwrap();
    }
    // Existing publication keys reject backslash. Their unchanged Unix
    // grammar already permits colon; the native-leaf API does not change it.
    assert!(owner.create_private_directory_exclusive(r"a\b").is_err());
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 0);
    #[cfg(unix)]
    {
        let created = owner.create_private_directory_exclusive("a:b").unwrap();
        assert_eq!(
            PublicationFiles::new(&root.0.join("a:b"))
                .unwrap()
                .root_identity(),
            created
        );
        assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
    }
    #[cfg(windows)]
    {
        assert!(owner.create_private_directory_exclusive("a:b").is_err());
        assert_eq!(fs::read_dir(&root.0).unwrap().count(), 0);
    }
    owner.check().unwrap();
}

#[cfg(unix)]
#[test]
fn native_private_leaf_preserves_literal_unix_backslash_and_colon() {
    let root = Directory::new();
    let mut owner = PublicationFiles::new(&root.0).unwrap();
    let name = r"native\leaf:literal";
    owner
        .create_private_native_directory_exclusive(std::ffi::OsStr::new(name))
        .unwrap();
    assert!(root.0.join(name).is_dir());
    assert!(!root.0.join("native").exists());
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
    owner.check().unwrap();
}

#[cfg(any(unix, windows))]
#[test]
fn native_private_leaf_refuses_non_utf8_names_without_creation() {
    let root = Directory::new();
    let mut owner = PublicationFiles::new(&root.0).unwrap();
    #[cfg(unix)]
    let name = {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec(b"native-\xff".to_vec())
    };
    #[cfg(windows)]
    let name = {
        use std::os::windows::ffi::OsStringExt;
        std::ffi::OsString::from_wide(&[0xd800])
    };
    let error = owner
        .create_private_native_directory_exclusive(&name)
        .unwrap_err();
    assert_eq!(error.kind, crate::ErrorKind::InvalidInput);
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 0);
    owner.check().unwrap();
}

#[cfg(unix)]
#[test]
fn native_private_leaf_refuses_a_replaced_retained_root() {
    let root = Directory::new();
    let selected = root.0.join("selected");
    let retained = root.0.join("retained");
    fs::create_dir(&selected).unwrap();
    let mut owner = PublicationFiles::new(&selected).unwrap();
    fs::rename(&selected, &retained).unwrap();
    fs::create_dir(&selected).unwrap();
    fs::write(selected.join("sentinel"), b"replacement bytes").unwrap();
    assert!(
        owner
            .create_private_native_directory_exclusive(std::ffi::OsStr::new("fresh"))
            .is_err()
    );
    assert_eq!(fs::read_dir(&retained).unwrap().count(), 0);
    assert!(!selected.join("fresh").exists());
    assert_eq!(
        fs::read(selected.join("sentinel")).unwrap(),
        b"replacement bytes"
    );
}

#[cfg(unix)]
#[test]
fn created_native_mode_capability_retains_raw_children_and_refreshes_only_owned_projection() {
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::PermissionsExt;
    let root = Directory::new();
    let mut owner = PublicationFiles::new(&root.0).unwrap();
    let mut target = owner
        .create_private_native_directory_capability(std::ffi::OsStr::new("target"))
        .unwrap();
    let raw = std::ffi::OsString::from_vec(b"empty-\\literal:\xff".to_vec());
    let mut child = target.create_private_child(&raw).unwrap();
    child.set_mode(0o710).unwrap();
    owner
        .set_created_native_directory_mode(&mut target, 0o750)
        .unwrap();
    owner.check().unwrap();
    target.check().unwrap();
    child.check().unwrap();
    assert_eq!(
        fs::metadata(root.0.join("target"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o750
    );
    assert_eq!(
        fs::metadata(root.0.join("target").join(&raw))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o710
    );
    let before = PublicationFiles::new(&root.0.join("target"))
        .unwrap()
        .root_identity();
    assert!(
        owner
            .set_created_native_directory_mode(&mut target, 0o10000)
            .is_err()
    );
    assert_eq!(
        PublicationFiles::new(&root.0.join("target"))
            .unwrap()
            .root_identity(),
        before
    );
}

#[cfg(unix)]
#[test]
fn created_native_mode_capability_refuses_a_foreign_target_replacement() {
    use std::os::unix::fs::PermissionsExt;
    let root = Directory::new();
    let selected = root.0.join("target");
    let aside = root.0.join("retained");
    let mut owner = PublicationFiles::new(&root.0).unwrap();
    let mut target = owner
        .create_private_native_directory_capability(std::ffi::OsStr::new("target"))
        .unwrap();
    fs::rename(&selected, &aside).unwrap();
    fs::create_dir(&selected).unwrap();
    fs::set_permissions(&selected, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(selected.join("sentinel"), b"foreign target bytes").unwrap();
    assert!(
        owner
            .set_created_native_directory_mode(&mut target, 0o700)
            .is_err()
    );
    assert_eq!(
        fs::metadata(&selected).unwrap().permissions().mode() & 0o7777,
        0o755
    );
    assert_eq!(
        fs::read(selected.join("sentinel")).unwrap(),
        b"foreign target bytes"
    );
    assert_eq!(
        fs::metadata(&aside).unwrap().permissions().mode() & 0o7777,
        0o700
    );
}
