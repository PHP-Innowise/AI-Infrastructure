//! Durable ownership handoff for native publication effects.
use super::{FileImageIdentity, Identity, Pin, PublicationFiles, native};
use crate::{Result, RuntimeError};
use serde_json::{Value, json as object};
use std::io::Write;
use std::{collections::BTreeMap, fs::File};
type RootedReceiptParent = (File, String, BTreeMap<String, (u64, u64, u32)>);

/// Rename committed, but no post-rename identity receipt could be obtained.
pub const POST_RENAME_IDENTITY_UNAVAILABLE: &str = "publication-post-rename-identity-unavailable";

/// One caller-admitted baseline; None in every field means the name must be absent.
#[derive(Clone, Copy)]
pub struct ExpectedImage<'a> {
    pub bytes: Option<&'a [u8]>,
    pub mode: Option<u32>,
    pub identity: Option<FileImageIdentity>,
}
/// Identity and permission projection captured from our held prepared file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PublishedImage {
    pub identity: FileImageIdentity,
    pub mode: u32,
}
pub enum WriteStage<'a> {
    Prepared(&'a Value),
    Published(PublishedImage),
}
fn image_tuple(image: &Identity) -> FileImageIdentity {
    (
        image.namespace.0,
        image.namespace.1,
        image.namespace.2,
        image.links,
        image.size,
        image.modified,
        image.changed,
    )
}
fn same_renamed_image(a: FileImageIdentity, b: FileImageIdentity) -> bool {
    (a.0, a.1, a.2, a.3, a.4, a.5) == (b.0, b.1, b.2, b.3, b.4, b.5)
}
fn held_mode(file: &File) -> Result<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Ok(file.metadata()?.permissions().mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        Ok(if file.metadata()?.permissions().readonly() {
            0o444
        } else {
            0o644
        })
    }
}
impl PublicationFiles {
    fn parent_owned(&mut self, relative: &str) -> Result<RootedReceiptParent> {
        let parts = relative.split('/').collect::<Vec<_>>();
        if relative.is_empty()
            || relative.starts_with('/')
            || relative.contains(['\\', '\0'])
            || parts
                .iter()
                .any(|part| part.is_empty() || *part == "." || *part == "..")
        {
            return Err(RuntimeError::policy("Invalid receipted publication path"));
        }
        let mut created = BTreeMap::new();
        for count in 1..parts.len() {
            let key = parts[..count]
                .iter()
                .map(|part| part.to_string())
                .collect::<Vec<_>>();
            if self.directories.contains_key(&key) {
                continue;
            }
            let previous = key[..key.len() - 1].to_vec();
            let parent = self
                .directories
                .get(&previous)
                .ok_or_else(|| RuntimeError::unavailable("Publication parent unavailable"))?
                .try_clone()?;
            let mut ours = false;
            let child = match native::directory(&parent, parts[count - 1]) {
                Ok(child) => child,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    self.check()?;
                    match native::mkdir(&parent, parts[count - 1]) {
                        Ok(()) => ours = true,
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(error) => return Err(error.into()),
                    }
                    native::directory(&parent, parts[count - 1])?
                }
                Err(error) => return Err(error.into()),
            };
            let identity = native::identity(&child)?.namespace;
            if ours {
                created.insert(key.join("/"), identity);
            }
            self.pins.push(Pin {
                parent,
                name: parts[count - 1].into(),
                child: child.try_clone()?,
                identity,
            });
            self.directories.insert(key, child);
        }
        self.check()?;
        let key = parts[..parts.len() - 1]
            .iter()
            .map(|part| part.to_string())
            .collect::<Vec<_>>();
        Ok((
            self.directories
                .get(&key)
                .ok_or_else(|| RuntimeError::unavailable("Publication parent missing"))?
                .try_clone()?,
            parts[parts.len() - 1].into(),
            created,
        ))
    }
    fn verify_reviewed_image(
        &mut self,
        relative: &str,
        expected: ExpectedImage<'_>,
        limit: usize,
    ) -> Result<()> {
        if expected.bytes.is_some() != expected.mode.is_some()
            || expected.bytes.is_some() != expected.identity.is_some()
        {
            return Err(RuntimeError::conflict(
                "Publication baseline image is inconsistent",
            ));
        }
        let bytes = self.read(relative, limit)?;
        let mode = self.read_mode(relative)?;
        // Identity is checked after the bounded byte/mode observations. Do not
        // replace the caller's reviewed identity with a newly observed inode.
        let identity = self.image_identity(relative)?;
        if bytes.as_deref() != expected.bytes
            || mode != expected.mode
            || identity != expected.identity
        {
            return Err(RuntimeError::conflict(
                "Publication reviewed identity, bytes or mode changed",
            ));
        }
        self.check()
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "Legacy receipted API separately binds bytes, mode, limit and callback"
    )]
    pub fn write_receipted<F>(
        &mut self,
        relative: &str,
        content: &[u8],
        expected: Option<&[u8]>,
        expected_mode: Option<u32>,
        mode: u32,
        limit: usize,
        before: F,
    ) -> Result<()>
    where
        F: FnOnce(&Value) -> Result<()>,
    {
        if limit > 512 * 1024 * 1024 || content.len() > limit || mode & !0o777 != 0 {
            return Err(RuntimeError::invalid(
                "Invalid receipted publication request",
            ));
        }
        let identity = self.image_identity(relative)?;
        let mut before = Some(before);
        self.write_reviewed_image(
            relative,
            content,
            ExpectedImage {
                bytes: expected,
                mode: expected_mode,
                identity,
            },
            mode,
            limit,
            |stage| match stage {
                WriteStage::Prepared(receipt) => {
                    let before = before
                        .take()
                        .ok_or_else(|| RuntimeError::conflict("Prepared callback replayed"))?;
                    before(receipt)
                }
                WriteStage::Published(_) => Ok(()),
            },
        )
        .map(|_| ())
    }
    pub fn write_reviewed_image<F>(
        &mut self,
        relative: &str,
        content: &[u8],
        expected: ExpectedImage<'_>,
        mode: u32,
        limit: usize,
        observe: F,
    ) -> Result<PublishedImage>
    where
        F: for<'stage> FnMut(WriteStage<'stage>) -> Result<()>,
    {
        self.write_reviewed_image_using(
            relative,
            content,
            expected,
            mode,
            limit,
            observe,
            native::identity,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Keep publication inputs unchanged while isolating the post-rename metadata read"
    )]
    fn write_reviewed_image_using<F, G>(
        &mut self,
        relative: &str,
        content: &[u8],
        expected: ExpectedImage<'_>,
        mode: u32,
        limit: usize,
        mut observe: F,
        read_published_identity: G,
    ) -> Result<PublishedImage>
    where
        F: for<'stage> FnMut(WriteStage<'stage>) -> Result<()>,
        G: FnOnce(&File) -> std::io::Result<Identity>,
    {
        if limit > 512 * 1024 * 1024 || content.len() > limit || mode & !0o777 != 0 {
            return Err(RuntimeError::invalid(
                "Invalid reviewed publication request",
            ));
        }
        self.verify_reviewed_image(relative, expected, limit)?;
        let (parent, name, created) = self.parent_owned(relative)?;
        let temporary = format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple());
        let temporary_relative = relative
            .rsplit_once('/')
            .map(|(parent, _)| format!("{parent}/{temporary}"))
            .unwrap_or_else(|| temporary.clone());
        let mut file = native::file(&parent, &temporary, true)?;
        let opened = native::identity(&file)?.namespace;
        let result = (|| -> Result<PublishedImage> {
            file.write_all(content)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(mode))?;
            }
            #[cfg(windows)]
            {
                let mut permissions = file.metadata()?.permissions();
                permissions.set_readonly(mode & 0o200 == 0);
                file.set_permissions(permissions)?;
            }
            file.flush()?;
            file.sync_all()?;
            let prepared_identity = native::identity(&file)?;
            let prepared = image_tuple(&prepared_identity);
            let output_mode = held_mode(&file)?;
            if prepared_identity.links != 1 || output_mode != super::kept_mode(mode) {
                return Err(RuntimeError::conflict(
                    "Prepared publication ownership or mode changed",
                ));
            }
            let receipt = object!({"identity":[prepared.0,prepared.1,prepared.2,prepared.3,prepared.4,prepared.5],"temporary":temporary_relative,"directories":self.directory_identities()?,"created":created});
            observe(WriteStage::Prepared(&receipt))?;
            self.check()?;
            self.verify_reviewed_image(relative, expected, limit)?;
            if native::identity(&file)? != prepared_identity
                || native::identity(&native::file(&parent, &temporary, false)?)?
                    != prepared_identity
            {
                return Err(RuntimeError::conflict(
                    "Prepared publication image was replaced",
                ));
            }
            native::rename(&parent, &temporary, &name, expected.bytes.is_none())?;
            // Obtain the receipt only from OUR descriptor. A query failure here
            // follows an actual committed rename: report retained evidence, never
            // substitute a current pathname inode or the pre-rename ctime.
            let committed = read_published_identity(&file).map_err(|source| {
                let mut error = RuntimeError::coded(
                    crate::ErrorKind::Io,
                    POST_RENAME_IDENTITY_UNAVAILABLE,
                    format!(
                        "Publication {relative} was renamed, but its held-file identity is unavailable; prepared identity {prepared:?}, mode {output_mode:o}; preserve the target for recovery: {source}"
                    ),
                );
                error.os_code = source.raw_os_error();
                error
            })?;
            let owned = PublishedImage {
                identity: image_tuple(&committed),
                mode: output_mode,
            };
            // Register the exact own-FD receipt before later sync/checks can fail.
            observe(WriteStage::Published(owned))?;
            #[cfg(unix)]
            parent.sync_all()?;
            self.check()?;
            if !same_renamed_image(owned.identity, prepared)
                || image_tuple(&native::identity(&file)?) != owned.identity
                || self.read(relative, limit)?.as_deref() != Some(content)
                || self.read_mode(relative)? != Some(output_mode)
                || self.image_identity(relative)? != Some(owned.identity)
            {
                return Err(RuntimeError::conflict(
                    "Published image differs from durable ownership receipt",
                ));
            }
            Ok(owned)
        })();
        // Keep foreign replacements at the temporary name, even with equal bytes.
        if native::file(&parent, &temporary, false)
            .ok()
            .filter(|named| named.metadata().is_ok_and(|metadata| metadata.is_file()))
            .and_then(|named| native::identity(&named).ok())
            .is_some_and(|current| {
                (current.namespace.0, current.namespace.1) == (opened.0, opened.1)
            })
        {
            let _ = native::unlink(&parent, &temporary);
        }
        result
    }
    pub fn remove_receipted<F>(
        &mut self,
        relative: &str,
        expected: &[u8],
        expected_mode: u32,
        limit: usize,
        before: F,
    ) -> Result<()>
    where
        F: FnMut(&Value) -> Result<()>,
    {
        let original = self
            .image_identity(relative)?
            .ok_or_else(|| RuntimeError::conflict("Removal baseline disappeared"))?;
        self.remove_reviewed_image(
            relative,
            ExpectedImage {
                bytes: Some(expected),
                mode: Some(expected_mode),
                identity: Some(original),
            },
            limit,
            before,
        )
    }
    pub fn remove_reviewed_image<F>(
        &mut self,
        relative: &str,
        expected: ExpectedImage<'_>,
        limit: usize,
        mut before: F,
    ) -> Result<()>
    where
        F: FnMut(&Value) -> Result<()>,
    {
        if limit > 512 * 1024 * 1024 {
            return Err(RuntimeError::invalid("Invalid reviewed removal limit"));
        }
        let original = expected
            .identity
            .ok_or_else(|| RuntimeError::conflict("Removal needs an owned image"))?;
        if expected.bytes.is_none() || expected.mode.is_none() {
            return Err(RuntimeError::conflict(
                "Removal baseline image is inconsistent",
            ));
        }
        self.verify_reviewed_image(relative, expected, limit)?;
        let (parent, name) = self.parent(relative, false)?;
        let quarantine = format!(".{name}.{}.removed", uuid::Uuid::new_v4().simple());
        let relative_quarantine = relative
            .rsplit_once('/')
            .map(|(parent, _)| format!("{parent}/{quarantine}"))
            .unwrap_or_else(|| quarantine.clone());
        before(
            &object!({"identity":original,"quarantine":relative_quarantine,"directories":self.directory_identities()?,"captured":false}),
        )?;
        self.check()?;
        self.verify_reviewed_image(relative, expected, limit)?;
        native::rename(&parent, &name, &quarantine, true)?;
        #[cfg(unix)]
        parent.sync_all()?;
        let captured = self.image_identity(&relative_quarantine)?;
        if !captured.is_some_and(|image| same_renamed_image(image, original))
            || self.read(&relative_quarantine, limit)?.as_deref() != expected.bytes
            || self.read_mode(&relative_quarantine)? != expected.mode
        {
            // An unexpected file moved by a concurrent rename stays intact.
            if self.read(relative, limit)?.is_none() {
                let _ = native::rename(&parent, &quarantine, &name, true);
            }
            return Err(RuntimeError::conflict(
                "Removal captured a different image; retained bytes require collision review",
            ));
        }
        let captured =
            captured.ok_or_else(|| RuntimeError::conflict("Removal image disappeared"))?;
        before(
            &object!({"identity":original,"quarantine":relative_quarantine,"directories":self.directory_identities()?,"captured":true}),
        )?;
        self.verify_reviewed_image(
            &relative_quarantine,
            ExpectedImage {
                identity: Some(captured),
                ..expected
            },
            limit,
        )?;
        self.check()?;
        native::unlink(&parent, &quarantine)?;
        #[cfg(unix)]
        parent.sync_all()?;
        self.check()
    }
}

#[cfg(all(test, unix))]
mod owned_temporary_cleanup_tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::PathBuf,
    };

    struct FixtureRoot(PathBuf);
    impl Drop for FixtureRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn failed_publication_preserves_a_foreign_same_byte_temporary_replacement() -> Result<()> {
        let path =
            std::env::temp_dir().join(format!("infra-owned-temporary-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path)?;
        let root = FixtureRoot(path);
        fs::set_permissions(&root.0, fs::Permissions::from_mode(0o700))?;
        let target = root.0.join("value.json");
        let retained = root.0.join("retained-owned.tmp");
        let before = b"original target\n";
        let prepared_bytes = b"prepared publication\n";
        fs::write(&target, before)?;
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644))?;
        let mut files = PublicationFiles::new(&root.0)?;
        let mut temporary = None;

        let result = files.write_receipted(
            "value.json",
            prepared_bytes,
            Some(before),
            Some(0o644),
            0o644,
            1024,
            |prepared| {
                let name = prepared["temporary"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("Prepared temporary name missing"))?;
                let named = root.0.join(name);
                let owned = fs::metadata(&named)?;
                assert!(owned.is_file());
                assert_eq!(fs::read(&named)?, prepared_bytes);
                assert_eq!(owned.permissions().mode() & 0o777, 0o644);
                fs::rename(&named, &retained)?;
                fs::write(&named, prepared_bytes)?;
                fs::set_permissions(&named, owned.permissions())?;
                let replacement = fs::metadata(&named)?;
                assert!(replacement.is_file());
                assert_eq!(replacement.permissions().mode() & 0o777, 0o644);
                assert_ne!(
                    (replacement.dev(), replacement.ino()),
                    (owned.dev(), owned.ino()),
                    "negative control must replace the actual file object"
                );
                temporary = Some(named);
                Err(RuntimeError::conflict("injected temporary-name takeover"))
            },
        );

        let error = result.expect_err("forced precommit refusal was lost");
        assert_eq!(error.kind, crate::ErrorKind::Conflict);
        assert!(error.message.contains("injected temporary-name takeover"));
        assert_eq!(fs::read(&target)?, before);
        let temporary = temporary.expect("actual prepared-publication callback did not run");
        assert_eq!(
            fs::read(&temporary)?,
            prepared_bytes,
            "cleanup deleted an unowned replacement at the temporary name"
        );
        assert_eq!(
            fs::metadata(&temporary)?.permissions().mode() & 0o777,
            0o644
        );
        assert_eq!(fs::read(&retained)?, prepared_bytes);
        Ok(())
    }

    fn swap_equal_test_image(
        root: &std::path::Path,
        relative: &str,
        retained: &std::path::Path,
    ) -> Result<FileImageIdentity> {
        let named = root.join(relative);
        let bytes = fs::read(&named)?;
        let permissions = fs::metadata(&named)?.permissions();
        let old = PublicationFiles::new(root)?
            .image_identity(relative)?
            .unwrap();
        fs::rename(&named, retained)?;
        fs::write(&named, &bytes)?;
        fs::set_permissions(&named, permissions)?;
        let replacement = PublicationFiles::new(root)?
            .image_identity(relative)?
            .unwrap();
        assert_ne!((old.0, old.1), (replacement.0, replacement.1));
        assert_eq!(fs::read(&named)?, bytes);
        Ok(replacement)
    }

    #[test]
    fn reviewed_write_rejects_equal_replacement_at_entry_and_after_prepare() -> Result<()> {
        for before_call in [true, false] {
            let outer = FixtureRoot(
                std::env::temp_dir().join(format!("infra-reviewed-write-{}", uuid::Uuid::new_v4())),
            );
            fs::create_dir(&outer.0)?;
            let root = outer.0.join("project");
            fs::create_dir(&root)?;
            let target = root.join("value.json");
            let retained = outer.0.join("retained-original");
            fs::write(&target, b"reviewed bytes\n")?;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o640))?;
            let mut files = PublicationFiles::new(&root)?;
            let identity = files.image_identity("value.json")?.unwrap();
            let expected = ExpectedImage {
                bytes: Some(b"reviewed bytes\n"),
                mode: Some(0o640),
                identity: Some(identity),
            };
            let mut replacement = if before_call {
                Some(swap_equal_test_image(&root, "value.json", &retained)?)
            } else {
                None
            };
            let result = files.write_reviewed_image(
                "value.json",
                b"new publication\n",
                expected,
                0o644,
                4096,
                |stage| {
                    if matches!(stage, WriteStage::Prepared(_)) && !before_call {
                        replacement = Some(swap_equal_test_image(&root, "value.json", &retained)?);
                    }
                    Ok(())
                },
            );
            assert_eq!(result.unwrap_err().kind, crate::ErrorKind::Conflict);
            assert_eq!(fs::read(&target)?, b"reviewed bytes\n");
            assert_eq!(fs::metadata(&target)?.permissions().mode() & 0o777, 0o640);
            assert_eq!(files.image_identity("value.json")?, replacement);
            assert_eq!(fs::read(&retained)?, b"reviewed bytes\n");
            for entry in fs::read_dir(&root)? {
                assert_eq!(
                    entry?.path(),
                    target,
                    "owned temporary must be cleaned after refusal"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn reviewed_removal_keeps_equal_foreign_replacement_at_the_quarantine_name() -> Result<()> {
        let outer = FixtureRoot(
            std::env::temp_dir().join(format!("infra-reviewed-removal-{}", uuid::Uuid::new_v4())),
        );
        fs::create_dir(&outer.0)?;
        let root = outer.0.join("project");
        fs::create_dir(&root)?;
        let retained = outer.0.join("retained-owned-image");
        let mut files = PublicationFiles::new(&root)?;
        let published = files.write_reviewed_image(
            "value.json",
            b"our publication\n",
            ExpectedImage {
                bytes: None,
                mode: None,
                identity: None,
            },
            0o644,
            4096,
            |_| Ok(()),
        )?;
        let mut quarantine = None;
        let mut replacement = None;
        let result = files.remove_reviewed_image(
            "value.json",
            ExpectedImage {
                bytes: Some(b"our publication\n"),
                mode: Some(published.mode),
                identity: Some(published.identity),
            },
            4096,
            |receipt| {
                if receipt["captured"] == true {
                    let relative = receipt["quarantine"].as_str().unwrap().to_string();
                    replacement = Some(swap_equal_test_image(&root, &relative, &retained)?);
                    quarantine = Some(relative);
                }
                Ok(())
            },
        );
        assert_eq!(result.unwrap_err().kind, crate::ErrorKind::Conflict);
        let quarantine = quarantine.unwrap();
        assert!(!root.join("value.json").exists());
        assert_eq!(fs::read(root.join(&quarantine))?, b"our publication\n");
        assert_eq!(files.image_identity(&quarantine)?, replacement);
        assert_eq!(fs::read(&retained)?, b"our publication\n");
        Ok(())
    }

    #[test]
    fn own_fd_receipt_is_observable_before_a_post_rename_failure() -> Result<()> {
        let outer = FixtureRoot(
            std::env::temp_dir().join(format!("infra-published-receipt-{}", uuid::Uuid::new_v4())),
        );
        fs::create_dir(&outer.0)?;
        let target = outer.0.join("value.json");
        fs::write(&target, b"original\n")?;
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640))?;
        let mut files = PublicationFiles::new(&outer.0)?;
        let original = files.image_identity("value.json")?.unwrap();
        let mut published = None;
        let result = files.write_reviewed_image(
            "value.json",
            b"published\n",
            ExpectedImage {
                bytes: Some(b"original\n"),
                mode: Some(0o640),
                identity: Some(original),
            },
            0o644,
            4096,
            |stage| {
                if let WriteStage::Published(image) = stage {
                    published = Some(image);
                    return Err(RuntimeError::conflict("injected post-rename refusal"));
                }
                Ok(())
            },
        );
        assert!(
            result
                .unwrap_err()
                .message
                .contains("injected post-rename refusal")
        );
        let published = published.expect("ownership must be registered before later failure");
        assert_eq!(
            files.image_identity("value.json")?,
            Some(published.identity)
        );
        files.write_reviewed_image(
            "value.json",
            b"original\n",
            ExpectedImage {
                bytes: Some(b"published\n"),
                mode: Some(published.mode),
                identity: Some(published.identity),
            },
            0o640,
            4096,
            |_| Ok(()),
        )?;
        assert_eq!(fs::read(&target)?, b"original\n");
        assert_eq!(fs::metadata(&target)?.permissions().mode() & 0o777, 0o640);
        Ok(())
    }
    #[test]
    fn post_rename_metadata_failure_preserves_unreceipted_effect_and_evidence() -> Result<()> {
        for (relative, original) in [
            ("payload.json", None),
            (
                "memory-bank/local/accelerator-install.json",
                Some(b"previous record\n".as_slice()),
            ),
        ] {
            for foreign_replacement in [false, true] {
                let outer = FixtureRoot(std::env::temp_dir().join(format!(
                    "infra-post-rename-metadata-{}",
                    uuid::Uuid::new_v4()
                )));
                fs::create_dir(&outer.0)?;
                let root = outer.0.join("project");
                fs::create_dir(&root)?;
                let target = root.join(relative);
                fs::create_dir_all(target.parent().unwrap())?;
                if let Some(bytes) = original {
                    fs::write(&target, bytes)?;
                    fs::set_permissions(&target, fs::Permissions::from_mode(0o640))?;
                }
                let retained = outer.0.join("retained-committed-owned-image");
                let mut files = PublicationFiles::new(&root)?;
                let expected = ExpectedImage {
                    bytes: original,
                    mode: original.map(|_| 0o640),
                    identity: files.image_identity(relative)?,
                };
                let mut prepared_receipt = None;
                let mut published = false;
                let mut replacement = None;
                let mut own_identity = None;
                let result = files.write_reviewed_image_using(
                    relative,
                    b"committed output\n",
                    expected,
                    0o644,
                    4096,
                    |stage| {
                        match stage {
                            WriteStage::Prepared(receipt) => {
                                prepared_receipt = Some(receipt.clone())
                            }
                            WriteStage::Published(_) => published = true,
                        }
                        Ok(())
                    },
                    |held| {
                        let own = image_tuple(&native::identity(held)?);
                        own_identity = Some(own);
                        assert_eq!(
                            fs::read(&target)?,
                            b"committed output\n",
                            "fault must occur after the actual rename"
                        );
                        if foreign_replacement {
                            replacement = Some(
                                swap_equal_test_image(&root, relative, &retained)
                                    .map_err(|error| std::io::Error::other(error.to_string()))?,
                            );
                        }
                        Err(std::io::Error::from_raw_os_error(libc::EIO))
                    },
                );
                let error = result.expect_err("post-rename metadata failure must propagate");
                assert_eq!(error.kind, crate::ErrorKind::Io);
                assert_eq!(
                    error.code.as_deref(),
                    Some(POST_RENAME_IDENTITY_UNAVAILABLE)
                );
                assert_eq!(error.os_code, Some(libc::EIO));
                assert!(error.message.contains(relative));
                assert!(error.message.contains("preserve the target for recovery"));
                assert!(!published, "no exact post-rename receipt was available");
                let own = own_identity.unwrap();
                let receipt = prepared_receipt.unwrap();
                assert_eq!(receipt["identity"][0], own.0);
                assert_eq!(receipt["identity"][1], own.1);
                assert_eq!(fs::read(&target)?, b"committed output\n");
                assert_eq!(fs::metadata(&target)?.permissions().mode() & 0o777, 0o644);
                if foreign_replacement {
                    assert_eq!(files.image_identity(relative)?, replacement);
                    assert_eq!(fs::read(&retained)?, b"committed output\n");
                    assert_ne!(
                        (replacement.unwrap().0, replacement.unwrap().1),
                        (own.0, own.1),
                    );
                } else {
                    assert_eq!(files.image_identity(relative)?, Some(own));
                }
            }
        }
        Ok(())
    }
}
