//! Read-only admission for explicitly selected native executable sources.
//! Cargo hardlinks are allowed here; data and installed-corpus readers stay strict.
use crate::{Result, RuntimeError, artifact};
use infra_core::bank::publication::{FileImageIdentity, PublicationFiles};
use std::path::{Path, PathBuf};

pub const LIMIT: usize = 512 * 1024 * 1024;

pub struct SelectedExecutableSource {
    path: PathBuf,
    name: String,
    held_parent: PublicationFiles,
    identity: FileImageIdentity,
    bytes: Vec<u8>,
    mode: u32,
    sha256: String,
}
fn typed(mut error: RuntimeError) -> RuntimeError {
    error.code = Some("eval-native-executable-source".into());
    error
}
fn refuse(message: &str) -> RuntimeError {
    typed(RuntimeError::policy(message))
}
impl SelectedExecutableSource {
    pub fn capture(path: &Path) -> Result<Self> {
        let path = artifact::ordinary(path).map_err(typed)?;
        let parent = path
            .parent()
            .ok_or_else(|| refuse("Selected executable has no parent"))?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| refuse("Selected executable name is not UTF-8"))?
            .to_string();
        let mut held_parent = PublicationFiles::new(parent).map_err(typed)?;
        let (bytes, identity) = held_parent
            .capture_source(&name, LIMIT)
            .map_err(typed)?
            .ok_or_else(|| refuse("Selected executable source is absent"))?;
        #[cfg(unix)]
        let mode = {
            let mode = identity.2 & 0o777;
            if mode & 0o111 == 0 || identity.2 & 0o7000 != 0 {
                return Err(refuse(
                    "Selected native executable needs ordinary executable permissions",
                ));
            }
            mode
        };
        #[cfg(not(unix))]
        let mode = if identity.2 & 0o200 == 0 {
            0o444
        } else {
            0o644
        };
        if !native_header(&bytes) {
            return Err(refuse(
                "Selected executable source has no admitted native image header",
            ));
        }
        let sha256 = artifact::sha(&bytes);
        let mut image = Self {
            path,
            name,
            held_parent,
            identity,
            bytes,
            mode,
            sha256,
        };
        image.recheck()?;
        Ok(image)
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn mode(&self) -> u32 {
        self.mode
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn identity(&self) -> FileImageIdentity {
        self.identity
    }
    /// Recheck through the same retained parent; never adopt a replacement image.
    pub fn recheck(&mut self) -> Result<()> {
        let (bytes, identity) = self
            .held_parent
            .capture_source(&self.name, LIMIT)
            .map_err(typed)?
            .ok_or_else(|| refuse("Selected executable source disappeared"))?;
        if identity != self.identity || bytes != self.bytes {
            return Err(typed(RuntimeError::conflict(
                "Selected executable identity or bytes changed",
            )));
        }
        Ok(())
    }
    pub fn into_bytes_and_mode(self) -> (Vec<u8>, u32) {
        (self.bytes, self.mode)
    }
}
pub fn read(path: &Path) -> Result<(Vec<u8>, u32)> {
    Ok(SelectedExecutableSource::capture(path)?.into_bytes_and_mode())
}
pub fn digest(path: &Path) -> Result<String> {
    Ok(SelectedExecutableSource::capture(path)?
        .sha256()
        .to_string())
}

/// An installer-selected engine is an executable source, not an ordinary resource.
/// Every other inventory member retains the caller's original data-file bound.
pub fn read_inventory_source(
    file: &infra_tooling::installer::InstalledFile,
    resource_limit: usize,
) -> Result<(Vec<u8>, u32)> {
    if [
        "memory-bank/bin/infra-engine",
        "memory-bank/bin/infra-engine.exe",
    ]
    .contains(&file.path.as_str())
    {
        if !file.executable || file.component != "shared" {
            return Err(refuse("Invalid selected native engine inventory role"));
        }
        return read(&file.source);
    }
    artifact::read_file(&file.source, resource_limit)
}

pub(crate) fn is_engine_slot(path: &str) -> bool {
    [
        "memory-bank/bin/infra-engine",
        "memory-bank/bin/infra-engine.exe",
    ]
    .contains(&path)
}

/// Byte-exact authority derived from the trusted installer canonical image.
/// This grants a separate raw-engine budget; it grants no general data exception.
pub struct CanonicalEngine<'a> {
    path: &'a str,
    bytes: &'a [u8],
    image_mode: u32,
}
impl<'a> CanonicalEngine<'a> {
    pub(crate) fn new(path: &'a str, bytes: &'a [u8], canonical_mode: u32) -> Result<Self> {
        if !is_engine_slot(path)
            || canonical_mode != 0o755
            || bytes.len() > LIMIT
            || !native_header(bytes)
        {
            return Err(refuse("Invalid canonical native engine authority"));
        }
        #[cfg(unix)]
        let image_mode = canonical_mode;
        #[cfg(not(unix))]
        let image_mode = 0o644;
        Ok(Self {
            path,
            bytes,
            image_mode,
        })
    }
    pub(crate) fn path(&self) -> &str {
        self.path
    }
    pub(crate) fn sha256(&self) -> String {
        artifact::sha(self.bytes)
    }
    pub(crate) fn matches(&self, path: &str, bytes: &[u8], mode: u32) -> bool {
        path == self.path && bytes == self.bytes && mode == self.image_mode
    }
}

fn u16_at(raw: &[u8], offset: usize, little: bool) -> Option<u16> {
    let bytes: [u8; 2] = raw.get(offset..offset.checked_add(2)?)?.try_into().ok()?;
    Some(if little {
        u16::from_le_bytes(bytes)
    } else {
        u16::from_be_bytes(bytes)
    })
}
fn u32_at(raw: &[u8], offset: usize, little: bool) -> Option<u32> {
    let bytes: [u8; 4] = raw.get(offset..offset.checked_add(4)?)?.try_into().ok()?;
    Some(if little {
        u32::from_le_bytes(bytes)
    } else {
        u32::from_be_bytes(bytes)
    })
}
fn u64_at(raw: &[u8], offset: usize, little: bool) -> Option<u64> {
    let bytes: [u8; 8] = raw.get(offset..offset.checked_add(8)?)?.try_into().ok()?;
    Some(if little {
        u64::from_le_bytes(bytes)
    } else {
        u64::from_be_bytes(bytes)
    })
}
fn elf(raw: &[u8]) -> bool {
    if !raw.starts_with(b"\x7fELF") || raw.len() < 52 || raw[6] != 1 {
        return false;
    }
    let little = match raw[5] {
        1 => true,
        2 => false,
        _ => return false,
    };
    let (size, size_offset) = match raw[4] {
        1 => (52, 40),
        2 => (64, 52),
        _ => return false,
    };
    raw.len() >= size
        && matches!(u16_at(raw, 16, little), Some(2 | 3))
        && u16_at(raw, 18, little).is_some_and(|machine| machine != 0)
        && u32_at(raw, 20, little) == Some(1)
        && u16_at(raw, size_offset, little) == Some(size as u16)
}
fn pe(raw: &[u8]) -> bool {
    if !raw.starts_with(b"MZ") {
        return false;
    }
    let Some(offset) = u32_at(raw, 0x3c, true).and_then(|n| usize::try_from(n).ok()) else {
        return false;
    };
    let Some(optional) = offset.checked_add(24) else {
        return false;
    };
    if offset < 64 || raw.get(offset..offset.saturating_add(4)) != Some(b"PE\0\0".as_slice()) {
        return false;
    }
    let Some(sections) = u16_at(raw, offset + 6, true) else {
        return false;
    };
    let Some(optional_size) = u16_at(raw, offset + 20, true) else {
        return false;
    };
    let Some(characteristics) = u16_at(raw, offset + 22, true) else {
        return false;
    };
    let Some(end) = optional
        .checked_add(optional_size as usize)
        .and_then(|n| n.checked_add(sections as usize * 40))
    else {
        return false;
    };
    sections > 0
        && optional_size >= 2
        && end <= raw.len()
        && characteristics & 2 != 0
        && characteristics & 0x2000 == 0
        && u16_at(raw, offset + 4, true).is_some_and(|machine| machine != 0)
        && matches!(u16_at(raw, optional, true), Some(0x10b | 0x20b))
}
fn thin_macho(raw: &[u8]) -> bool {
    let Some(magic) = raw.get(..4) else {
        return false;
    };
    let (little, size) = match magic {
        [0xce, 0xfa, 0xed, 0xfe] => (true, 28),
        [0xcf, 0xfa, 0xed, 0xfe] => (true, 32),
        [0xfe, 0xed, 0xfa, 0xce] => (false, 28),
        [0xfe, 0xed, 0xfa, 0xcf] => (false, 32),
        _ => return false,
    };
    raw.len() >= size
        && u32_at(raw, 12, little) == Some(2)
        && u32_at(raw, 20, little)
            .and_then(|n| usize::try_from(n).ok())
            .and_then(|n| size.checked_add(n))
            .is_some_and(|end| end <= raw.len())
}
fn fat_macho(raw: &[u8]) -> bool {
    let Some(magic) = raw.get(..4) else {
        return false;
    };
    let (little, wide) = match magic {
        [0xca, 0xfe, 0xba, 0xbe] => (false, false),
        [0xbe, 0xba, 0xfe, 0xca] => (true, false),
        [0xca, 0xfe, 0xba, 0xbf] => (false, true),
        [0xbf, 0xba, 0xfe, 0xca] => (true, true),
        _ => return false,
    };
    let Some(count) = u32_at(raw, 4, little).filter(|n| (1..=64).contains(n)) else {
        return false;
    };
    let stride = if wide { 32 } else { 20 };
    let table_end = 8 + count as usize * stride;
    if table_end > raw.len() {
        return false;
    }
    for index in 0..count as usize {
        let row = 8 + index * stride;
        let offset = if wide {
            u64_at(raw, row + 8, little)
        } else {
            u32_at(raw, row + 8, little).map(u64::from)
        };
        let length = if wide {
            u64_at(raw, row + 16, little)
        } else {
            u32_at(raw, row + 12, little).map(u64::from)
        };
        let Some((offset, length)) = offset.zip(length).and_then(|(offset, length)| {
            Some((usize::try_from(offset).ok()?, usize::try_from(length).ok()?))
        }) else {
            return false;
        };
        let Some(end) = offset.checked_add(length) else {
            return false;
        };
        if offset < table_end || end > raw.len() || !thin_macho(&raw[offset..end]) {
            return false;
        }
    }
    true
}
fn native_header(raw: &[u8]) -> bool {
    elf(raw) || pe(raw) || thin_macho(raw) || fat_macho(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    struct Owned(PathBuf);
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> Owned {
        let path = std::env::temp_dir().join(format!(
            "infra-selected-executable-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&path).unwrap();
        artifact::chmod(&path, 0o700).unwrap();
        Owned(path)
    }
    #[test]
    fn hardlinked_native_source_is_explicit_and_replacement_is_not_adopted() {
        let owned = fixture();
        let source = owned.0.join("selected");
        let second = owned.0.join("second-name");
        fs::copy(std::env::current_exe().unwrap(), &source).unwrap();
        artifact::chmod(&source, 0o700).unwrap();
        fs::hard_link(&source, &second).unwrap();
        let mut selected = SelectedExecutableSource::capture(&source).unwrap();
        assert_eq!(selected.identity().3, 2);
        assert_eq!(selected.sha256(), artifact::sha(selected.bytes()));
        #[cfg(unix)]
        assert!(artifact::read_file(&source, LIMIT).is_err());
        let retained = owned.0.join("retained-original");
        fs::rename(&source, &retained).unwrap();
        fs::copy(&second, &source).unwrap();
        artifact::chmod(&source, 0o700).unwrap();
        let replacement = SelectedExecutableSource::capture(&source).unwrap();
        assert_ne!(
            (selected.identity().0, selected.identity().1),
            (replacement.identity().0, replacement.identity().1),
        );
        assert_eq!(selected.sha256(), replacement.sha256());
        assert!(selected.recheck().is_err());
        assert_eq!(digest(&retained).unwrap(), selected.sha256());
        assert_eq!(digest(&source).unwrap(), selected.sha256());
    }
    #[test]
    fn nonnative_oversize_and_nonexecutable_sources_are_refused() {
        let owned = fixture();
        let source = owned.0.join("selected");
        fs::write(&source, b"#!/bin/sh\nexit 0\n").unwrap();
        artifact::chmod(&source, 0o700).unwrap();
        assert!(SelectedExecutableSource::capture(&source).is_err());
        let file = fs::OpenOptions::new().write(true).open(&source).unwrap();
        file.set_len(LIMIT as u64 + 1).unwrap();
        assert!(SelectedExecutableSource::capture(&source).is_err());
        drop(file);
        fs::remove_file(&source).unwrap();
        fs::copy(std::env::current_exe().unwrap(), &source).unwrap();
        #[cfg(unix)]
        {
            artifact::chmod(&source, 0o600).unwrap();
            assert!(SelectedExecutableSource::capture(&source).is_err());
            artifact::chmod(&source, 0o700).unwrap();
            let link = owned.0.join("linked-executable");
            std::os::unix::fs::symlink(&source, &link).unwrap();
            assert!(SelectedExecutableSource::capture(&link).is_err());
            let parent_link = owned.0.join("linked-parent");
            std::os::unix::fs::symlink(&owned.0, &parent_link).unwrap();
            assert!(SelectedExecutableSource::capture(&parent_link.join("selected")).is_err());
        }
    }
    #[test]
    fn only_shared_executable_engine_inventory_slots_use_the_native_source_bound() {
        let owned = fixture();
        let source = owned.0.join("selected");
        fs::copy(std::env::current_exe().unwrap(), &source).unwrap();
        artifact::chmod(&source, 0o700).unwrap();
        fs::hard_link(&source, owned.0.join("second-name")).unwrap();
        let expected = digest(&source).unwrap();
        for path in [
            "memory-bank/bin/infra-engine",
            "memory-bank/bin/infra-engine.exe",
        ] {
            let file = infra_tooling::installer::InstalledFile {
                path: path.into(),
                source: source.clone(),
                executable: true,
                component: "shared".into(),
            };
            // Even a one-byte resource ceiling cannot be used as the native
            // engine ceiling: this role has its separate explicit 512MiB bound.
            let (bytes, _) = read_inventory_source(&file, 1).unwrap();
            assert!(bytes.len() > 1);
            assert_eq!(artifact::sha(&bytes), expected);
        }
        let changed_flag = infra_tooling::installer::InstalledFile {
            path: "memory-bank/bin/infra-engine".into(),
            source: source.clone(),
            executable: false,
            component: "shared".into(),
        };
        assert!(read_inventory_source(&changed_flag, LIMIT).is_err());
        let changed_component = infra_tooling::installer::InstalledFile {
            executable: true,
            component: "codex".into(),
            ..changed_flag
        };
        assert!(read_inventory_source(&changed_component, LIMIT).is_err());
        let other_executable = infra_tooling::installer::InstalledFile {
            path: "assets/other-executable".into(),
            source: source.clone(),
            executable: true,
            component: "shared".into(),
        };
        assert!(
            read_inventory_source(&other_executable, 1).is_err(),
            "an executable flag alone must not select the native source policy"
        );
        #[cfg(unix)]
        assert!(
            read_inventory_source(&other_executable, LIMIT).is_err(),
            "other executable resources still require ordinary one-link admission"
        );
        assert_eq!(digest(&source).unwrap(), expected);
    }

    #[test]
    fn ordinary_inventory_data_keeps_its_original_reader_and_limit() {
        let owned = fixture();
        let source = owned.0.join("resource");
        fs::write(&source, b"ordinary resource\n").unwrap();
        artifact::chmod(&source, 0o600).unwrap();
        let file = infra_tooling::installer::InstalledFile {
            path: "memory-bank/config/ordinary.json".into(),
            source: source.clone(),
            executable: false,
            component: "shared".into(),
        };
        assert_eq!(
            read_inventory_source(&file, 64).unwrap().0,
            b"ordinary resource\n"
        );
        assert!(read_inventory_source(&file, 1).is_err());
        fs::hard_link(&source, owned.0.join("second-resource-name")).unwrap();
        #[cfg(unix)]
        assert!(
            read_inventory_source(&file, 64).is_err(),
            "resource hardlinks must not inherit the selected engine exception"
        );
    }
}
