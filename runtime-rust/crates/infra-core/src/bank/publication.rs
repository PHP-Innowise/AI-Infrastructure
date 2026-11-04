//! Rooted directory custody and exact-image compare-and-swap publication.
pub mod archive;
pub mod commit;
pub mod cursor;
pub mod observe;
pub mod private;
pub mod trace;
pub mod walk;
use crate::{Result, RuntimeError, domain::json};
use serde_json::{Value, json as object};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const IMAGE_LIMIT: usize = 1024 * 1024;
pub const PLAN_LIMIT: usize = 4 * 1024 * 1024;
pub type FileImageIdentity = (u64, u64, u32, u64, u64, i128, i128);
pub type CapturedFileImage = (Vec<u8>, FileImageIdentity);
#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    namespace: (u64, u64, u32),
    links: u64,
    size: u64,
    modified: i128,
    changed: i128,
}
struct Pin {
    parent: File,
    name: String,
    child: File,
    identity: (u64, u64, u32),
}
pub struct PublicationFiles {
    root: PathBuf,
    root_identity: (u64, u64, u32),
    directories: BTreeMap<Vec<String>, File>,
    pins: Vec<Pin>,
}

impl PublicationFiles {
    pub fn new(root: &Path) -> Result<Self> {
        let root = crate::fs::absolute(root)?;
        let file = native::root(&root)?;
        let identity = native::identity(&file)?;
        let mut directories = BTreeMap::new();
        directories.insert(vec![], file);
        let result = Self {
            root,
            root_identity: identity.namespace,
            directories,
            pins: vec![],
        };
        result.check()?;
        Ok(result)
    }
    pub fn check(&self) -> Result<()> {
        if native::identity(&native::root(&self.root)?)?.namespace != self.root_identity {
            return Err(RuntimeError::conflict("Publication root custody changed"));
        }
        for pin in &self.pins {
            let named = native::directory(&pin.parent, &pin.name)?;
            if native::identity(&named)?.namespace != pin.identity
                || native::identity(&pin.child)?.namespace != pin.identity
            {
                return Err(RuntimeError::conflict(
                    "Publication directory custody changed",
                ));
            }
        }
        Ok(())
    }
    fn parent(&mut self, relative: &str, create: bool) -> Result<(File, String)> {
        let parts = relative.split('/').collect::<Vec<_>>();
        if relative.is_empty()
            || relative.starts_with('/')
            || relative.contains('\\')
            || relative.contains('\0')
            || parts
                .iter()
                .any(|part| part.is_empty() || *part == "." || *part == "..")
            || Path::new(relative).is_absolute()
        {
            return Err(RuntimeError::policy("Invalid publication path"));
        }
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
            let child = match native::directory(&parent, parts[count - 1]) {
                Ok(file) => file,
                Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
                    self.check()?;
                    match native::mkdir(&parent, parts[count - 1]) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(error) => return Err(error.into()),
                    }
                    native::directory(&parent, parts[count - 1])?
                }
                Err(error) => return Err(error.into()),
            };
            let identity = native::identity(&child)?.namespace;
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
                .ok_or_else(|| RuntimeError::unavailable("Publication parent unavailable"))?
                .try_clone()?,
            parts[parts.len() - 1].into(),
        ))
    }
    pub fn read(&mut self, relative: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        self.read_checked(relative, limit, |_| Ok(()))
    }
    pub fn read_checked<F>(
        &mut self,
        relative: &str,
        limit: usize,
        before: F,
    ) -> Result<Option<Vec<u8>>>
    where
        F: FnOnce(u64) -> Result<()>,
    {
        let (parent, name) = match self.parent(relative, false) {
            Ok(parent) => parent,
            Err(error) if error.kind == crate::ErrorKind::NotFound => {
                self.check()?;
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let mut file = match native::file(&parent, &name, false) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.check()?;
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        let identity = native::identity(&file)?;
        if !file.metadata()?.is_file() || identity.links != 1 {
            return Err(RuntimeError::policy(
                "Publication image is not an ordinary bounded file",
            ));
        }
        self.check()?;
        before(identity.size)?;
        if identity.size > limit as u64 {
            return Err(RuntimeError::policy(
                "Publication image exceeds its read limit",
            ));
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)?;
        let named = native::file(&parent, &name, false)?;
        if bytes.len() > limit
            || native::identity(&file)? != identity
            || native::identity(&named)? != identity
        {
            return Err(RuntimeError::conflict(
                "Publication image changed during read",
            ));
        }
        self.check()?;
        Ok(Some(bytes))
    }
    pub fn write_expected(
        &mut self,
        relative: &str,
        content: &[u8],
        expected: Option<&[u8]>,
    ) -> Result<()> {
        self.write_expected_limit(relative, content, expected, 2 * PLAN_LIMIT)
    }
    pub fn write_expected_limit(
        &mut self,
        relative: &str,
        content: &[u8],
        expected: Option<&[u8]>,
        limit: usize,
    ) -> Result<()> {
        if limit > 512 * 1024 * 1024
            || content.len() > limit
            || self.read(relative, limit)?.as_deref() != expected
        {
            return Err(RuntimeError::conflict(
                "Publication effect changed before write",
            ));
        }
        let (parent, name) = self.parent(relative, true)?;
        let temporary = format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple());
        let mut file = native::file(&parent, &temporary, true)?;
        let identity = native::identity(&file)?.namespace;
        let result = (|| -> Result<()> {
            file.write_all(content)?;
            file.flush()?;
            file.sync_all()?;
            self.check()?;
            if self.read(relative, limit)?.as_deref() != expected {
                return Err(RuntimeError::conflict(
                    "Publication effect changed before commit",
                ));
            }
            native::rename(&parent, &temporary, &name, expected.is_none())?;
            #[cfg(unix)]
            parent.sync_all()?;
            self.check()?;
            if self.read(relative, limit)?.as_deref() != Some(content) {
                return Err(RuntimeError::conflict(
                    "Publication effect differs after write",
                ));
            }
            Ok(())
        })();
        if native::file(&parent, &temporary, false)
            .ok()
            .and_then(|file| native::identity(&file).ok())
            .is_some_and(|current| current.namespace == identity)
        {
            let _ = native::unlink(&parent, &temporary);
        }
        result
    }
    pub fn set_mode(&mut self, relative: &str, mode: u32) -> Result<()> {
        let (parent, name) = self.parent(relative, false)?;
        let file = native::file(&parent, &name, false)?;
        let identity = native::identity(&file)?;
        if !file.metadata()?.is_file() || identity.links != 1 {
            return Err(RuntimeError::policy(
                "Mode destination is not an ordinary owned file",
            ));
        }
        self.check()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(mode & 0o777))?;
        }
        #[cfg(windows)]
        {
            let mut permissions = file.metadata()?.permissions();
            permissions.set_readonly(mode & 0o200 == 0);
            file.set_permissions(permissions)?;
        }
        self.check()?;
        if native::identity(&native::file(&parent, &name, false)?)?.namespace != identity.namespace
        {
            return Err(RuntimeError::conflict("Mode destination changed"));
        }
        Ok(())
    }
    pub fn read_mode(&mut self, relative: &str) -> Result<Option<u32>> {
        let (parent, name) = match self.parent(relative, false) {
            Ok(value) => value,
            Err(error) if error.kind == crate::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let file = match native::file(&parent, &name, false) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let identity = native::identity(&file)?;
        if !file.metadata()?.is_file() || identity.links != 1 {
            return Err(RuntimeError::policy(
                "Mode source is not an ordinary owned file",
            ));
        }
        self.check()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            Ok(Some(file.metadata()?.permissions().mode() & 0o777))
        }
        #[cfg(not(unix))]
        {
            Ok(Some(if file.metadata()?.permissions().readonly() {
                0o444
            } else {
                0o644
            }))
        }
    }
    pub fn unlink_expected_limit(
        &mut self,
        relative: &str,
        expected: &[u8],
        limit: usize,
    ) -> Result<()> {
        if limit > 512 * 1024 * 1024 || self.read(relative, limit)?.as_deref() != Some(expected) {
            return Err(RuntimeError::conflict(
                "Publication effect changed before removal",
            ));
        }
        let (parent, name) = self.parent(relative, false)?;
        self.check()?;
        native::unlink(&parent, &name)?;
        #[cfg(unix)]
        parent.sync_all()?;
        self.check()
    }
    pub fn write_expected_mode(
        &mut self,
        relative: &str,
        content: &[u8],
        expected: Option<&[u8]>,
        mode: u32,
        limit: usize,
    ) -> Result<()> {
        let expected_mode = self.read_mode(relative)?;
        self.write_expected_mode_checked(relative, content, expected, expected_mode, mode, limit)
    }
    pub fn write_expected_mode_checked(
        &mut self,
        relative: &str,
        content: &[u8],
        expected: Option<&[u8]>,
        expected_mode: Option<u32>,
        mode: u32,
        limit: usize,
    ) -> Result<()> {
        if limit > 512 * 1024 * 1024 || content.len() > limit || mode & !0o777 != 0 {
            return Err(RuntimeError::invalid(
                "Invalid bounded publication mode request",
            ));
        }
        if self.read(relative, limit)?.as_deref() != expected
            || self.read_mode(relative)? != expected_mode
        {
            return Err(RuntimeError::conflict(
                "Publication bytes or mode changed before write",
            ));
        }
        let (parent, name) = self.parent(relative, true)?;
        let temporary = format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple());
        let mut file = native::file(&parent, &temporary, true)?;
        let identity = native::identity(&file)?.namespace;
        let result = (|| -> Result<()> {
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
            self.check()?;
            if self.read(relative, limit)?.as_deref() != expected
                || self.read_mode(relative)? != expected_mode
            {
                return Err(RuntimeError::conflict(
                    "Publication bytes or mode changed before commit",
                ));
            }
            native::rename(&parent, &temporary, &name, expected.is_none())?;
            #[cfg(unix)]
            parent.sync_all()?;
            self.check()?;
            if self.read(relative, limit)?.as_deref() != Some(content) {
                return Err(RuntimeError::conflict(
                    "Publication effect differs after mode-preserving write",
                ));
            }
            Ok(())
        })();
        // Our chmod changes the mode projection, not ownership. The original
        // file handle remains live, so its device/file ID cannot be recycled.
        if native::file(&parent, &temporary, false)
            .ok()
            .filter(|named| named.metadata().is_ok_and(|metadata| metadata.is_file()))
            .and_then(|named| native::identity(&named).ok())
            .is_some_and(|current| {
                (current.namespace.0, current.namespace.1) == (identity.0, identity.1)
            })
        {
            let _ = native::unlink(&parent, &temporary);
        }
        result
    }
    /// Create one previously absent directory beneath held parent descriptors.
    /// Mode is subject to the process umask, as with ordinary directory creation.
    pub fn create_directory_exclusive(
        &mut self,
        relative: &str,
        mode: u32,
    ) -> Result<(u64, u64, u32)> {
        if mode & !0o777 != 0 {
            return Err(RuntimeError::invalid("Invalid new directory mode"));
        }
        let (parent, name) = self.parent(relative, false)?;
        self.check()?;
        native::mkdir_mode(&parent, &name, mode)?;
        let child = native::directory(&parent, &name)?;
        let identity = native::identity(&child)?.namespace;
        let key = relative.split('/').map(str::to_owned).collect::<Vec<_>>();
        self.pins.push(Pin {
            parent: parent.try_clone()?,
            name,
            child: child.try_clone()?,
            identity,
        });
        self.directories.insert(key, child);
        #[cfg(unix)]
        parent.sync_all()?;
        self.check()?;
        Ok(identity)
    }
    /// Remove only the same empty directory that this caller created.
    pub fn remove_empty_directory_expected(
        &mut self,
        relative: &str,
        expected: (u64, u64, u32),
    ) -> Result<bool> {
        let (parent, name) = match self.parent(relative, false) {
            Ok(value) => value,
            Err(error) if error.kind == crate::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        let child = match native::directory(&parent, &name) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        if native::identity(&child)?.namespace != expected {
            return Err(RuntimeError::conflict(
                "Created install directory was replaced",
            ));
        }
        self.check()?;
        self.archive_remove_empty_directory(relative)
    }
    pub fn root_identity(&self) -> (u64, u64, u32) {
        self.root_identity
    }
    pub fn directory_identities(&self) -> Result<BTreeMap<String, (u64, u64, u32)>> {
        self.check()?;
        self.directories
            .iter()
            .map(|(parts, file)| Ok((parts.join("/"), native::identity(file)?.namespace)))
            .collect()
    }
    pub fn check_directory_identities(
        &mut self,
        expected: &BTreeMap<String, (u64, u64, u32)>,
    ) -> Result<()> {
        if expected.get("") != Some(&self.root_identity) {
            return Err(RuntimeError::conflict("Preview root identity differs"));
        }
        for (relative, wanted) in expected {
            let file = if relative.is_empty() {
                self.directories
                    .get(&Vec::<String>::new())
                    .ok_or_else(|| RuntimeError::unavailable("Publication root is unavailable"))?
                    .try_clone()?
            } else {
                self.parent(&format!("{relative}/.custody-probe"), false)?.0
            };
            if native::identity(&file)?.namespace != *wanted {
                return Err(RuntimeError::conflict("Preview directory identity differs"));
            }
        }
        self.check()
    }
    pub fn image_identity(&mut self, relative: &str) -> Result<Option<FileImageIdentity>> {
        let (parent, name) = match self.parent(relative, false) {
            Ok(value) => value,
            Err(error) if error.kind == crate::ErrorKind::NotFound => {
                self.check()?;
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let file = match native::file(&parent, &name, false) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.check()?;
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        let current = native::identity(&file)?;
        if !file.metadata()?.is_file() || current.links != 1 {
            return Err(RuntimeError::policy(
                "Preview image is not an ordinary owned file",
            ));
        }
        if native::identity(&native::file(&parent, &name, false)?)? != current {
            return Err(RuntimeError::conflict("Preview image identity changed"));
        }
        self.check()?;
        Ok(Some((
            current.namespace.0,
            current.namespace.1,
            current.namespace.2,
            current.links,
            current.size,
            current.modified,
            current.changed,
        )))
    }
    pub fn read_prefix(&mut self, relative: &str, limit: usize) -> Result<Option<(u64, Vec<u8>)>> {
        if limit > 512 * 1024 * 1024 {
            return Err(RuntimeError::invalid(
                "Prefix read limit exceeds the native bound",
            ));
        }
        let (parent, name) = match self.parent(relative, false) {
            Ok(value) => value,
            Err(error) if error.kind == crate::ErrorKind::NotFound => {
                self.check()?;
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let mut file = match native::file(&parent, &name, false) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.check()?;
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        let identity = native::identity(&file)?;
        if !file.metadata()?.is_file() || identity.links != 1 {
            return Err(RuntimeError::policy(
                "Prefix source is not an ordinary owned file",
            ));
        }
        self.check()?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(limit as u64)
            .read_to_end(&mut bytes)?;
        if native::identity(&file)? != identity
            || native::identity(&native::file(&parent, &name, false)?)? != identity
        {
            return Err(RuntimeError::conflict(
                "Prefix source changed during capture",
            ));
        }
        self.check()?;
        Ok(Some((identity.size, bytes)))
    }
    pub fn rename_expected_same_parent(
        &mut self,
        from: &str,
        to: &str,
        expected: &[u8],
        limit: usize,
    ) -> Result<()> {
        if limit > 512 * 1024 * 1024
            || from == to
            || from.rsplit_once('/').map(|(parent, _)| parent)
                != to.rsplit_once('/').map(|(parent, _)| parent)
        {
            return Err(RuntimeError::invalid(
                "Atomic queue claim requires distinct names in one directory",
            ));
        }
        if self.read(from, limit)?.as_deref() != Some(expected) || self.read(to, limit)?.is_some() {
            return Err(RuntimeError::conflict(
                "Queue claim source or destination changed",
            ));
        }
        let (source_parent, source) = self.parent(from, false)?;
        let (target_parent, target) = self.parent(to, false)?;
        if native::identity(&source_parent)?.namespace
            != native::identity(&target_parent)?.namespace
        {
            return Err(RuntimeError::conflict(
                "Queue claim directory custody differs",
            ));
        }
        let original = native::identity(&native::file(&source_parent, &source, false)?)?;
        self.check()?;
        if self.read(from, limit)?.as_deref() != Some(expected)
            || self.read(to, limit)?.is_some()
            || native::identity(&native::file(&source_parent, &source, false)?)? != original
        {
            return Err(RuntimeError::conflict(
                "Queue claim changed before atomic rename",
            ));
        }
        native::rename(&source_parent, &source, &target, true)?;
        #[cfg(unix)]
        source_parent.sync_all()?;
        self.check()?;
        if self.read(from, limit)?.is_some() || self.read(to, limit)?.as_deref() != Some(expected) {
            return Err(RuntimeError::conflict(
                "Atomic queue claim differs after rename",
            ));
        }
        self.check()
    }
    pub fn unlink_expected(&mut self, relative: &str, expected: &[u8]) -> Result<()> {
        if self.read(relative, 2 * PLAN_LIMIT)?.as_deref() != Some(expected) {
            return Err(RuntimeError::conflict(
                "Publication effect changed before removal",
            ));
        }
        let (parent, name) = self.parent(relative, false)?;
        self.check()?;
        native::unlink(&parent, &name)?;
        #[cfg(unix)]
        parent.sync_all()?;
        self.check()
    }
}

/// The mode `read_mode` reports for a file published with `mode`. Windows
/// keeps only read-only, which publication sets from the owner's write bit,
/// so every mode reads back there as 0o444 or 0o644.
pub fn kept_mode(mode: u32) -> u32 {
    if cfg!(unix) {
        mode
    } else if mode & 0o200 == 0 {
        0o444
    } else {
        0o644
    }
}
pub fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
pub fn digest(value: &Value) -> Result<String> {
    Ok(sha256(json::canonical(value)?.as_bytes()))
}
pub fn seal(value: &mut Value) -> Result<()> {
    let mut copy = value.clone();
    copy.as_object_mut()
        .ok_or_else(|| RuntimeError::invalid("Custody plan must be an object"))?
        .remove("sha256");
    value["sha256"] = digest(&copy)?.into();
    Ok(())
}
pub fn verify_seal(value: &Value) -> Result<()> {
    let mut expected = value.clone();
    seal(&mut expected)?;
    if expected["sha256"] != value["sha256"] {
        return Err(RuntimeError::invalid("Custody plan seal differs"));
    }
    Ok(())
}
pub fn blob(bytes: &[u8]) -> Result<Value> {
    if bytes.len() > IMAGE_LIMIT {
        return Err(RuntimeError::invalid("Publication image limit exceeded"));
    }
    Ok(object!({"sha256":sha256(bytes),"data":base64_encode(bytes)}))
}
pub fn unblob(value: &Value) -> Result<Vec<u8>> {
    if value.as_object().is_none_or(|o| o.len() != 2) || !value["sha256"].is_string() {
        return Err(RuntimeError::invalid("Invalid publication blob"));
    }
    let text = value["data"]
        .as_str()
        .ok_or_else(|| RuntimeError::invalid("Invalid publication encoding"))?;
    if text.len() > IMAGE_LIMIT.div_ceil(3) * 4 {
        return Err(RuntimeError::invalid("Publication image limit exceeded"));
    }
    let bytes = base64_decode(text)?;
    if bytes.len() > IMAGE_LIMIT
        || std::str::from_utf8(&bytes).is_err()
        || sha256(&bytes) != value["sha256"].as_str().unwrap_or("")
    {
        return Err(RuntimeError::invalid(
            "Publication image checksum or encoding differs",
        ));
    }
    Ok(bytes)
}
fn base64_encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let a = chunk[0] as usize;
        let b = chunk.get(1).copied().unwrap_or(0) as usize;
        let c = chunk.get(2).copied().unwrap_or(0) as usize;
        out.push(DIGITS[a >> 2] as char);
        out.push(DIGITS[((a & 3) << 4) | (b >> 4)] as char);
        out.push(if chunk.len() > 1 {
            DIGITS[((b & 15) << 2) | (c >> 6)] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            DIGITS[c & 63] as char
        } else {
            '='
        });
    }
    out
}
fn base64_decode(text: &str) -> Result<Vec<u8>> {
    let bad = || RuntimeError::invalid("Invalid publication base64");
    if !text.len().is_multiple_of(4) {
        return Err(bad());
    }
    let mut out = Vec::new();
    for (i, part) in text.as_bytes().chunks(4).enumerate() {
        let mut n = [0u8; 4];
        for j in 0..4 {
            n[j] = match part[j] {
                b'A'..=b'Z' => part[j] - b'A',
                b'a'..=b'z' => part[j] - b'a' + 26,
                b'0'..=b'9' => part[j] - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' if j >= 2 && i == text.len() / 4 - 1 => 0,
                _ => return Err(bad()),
            };
        }
        if part[2] == b'=' && part[3] != b'='
            || part[2] == b'=' && n[1] & 15 != 0
            || part[3] == b'=' && part[2] != b'=' && n[2] & 3 != 0
        {
            return Err(bad());
        }
        out.push((n[0] << 2) | (n[1] >> 4));
        if part[2] != b'=' {
            out.push((n[1] << 4) | (n[2] >> 2));
        }
        if part[3] != b'=' {
            out.push((n[2] << 6) | n[3]);
        }
    }
    Ok(out)
}

#[cfg(unix)]
mod native {
    use super::{File, Identity, Path};
    use std::{
        ffi::CString,
        io,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::MetadataExt,
        },
    };
    fn name(value: &str) -> io::Result<CString> {
        CString::new(value)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in rooted name"))
    }
    fn open(parent: i32, value: &str, flags: i32, mode: u32) -> io::Result<File> {
        let value = name(value)?;
        let fd = unsafe { libc::openat(parent, value.as_ptr(), flags, mode) };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    pub fn root(path: &Path) -> io::Result<File> {
        let mut current = File::open("/")?;
        for component in path.components() {
            if let std::path::Component::Normal(name) = component {
                use std::os::unix::ffi::OsStrExt;
                let name = CString::new(name.as_bytes())
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in root"))?;
                let fd = unsafe {
                    libc::openat(
                        current.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
                if fd < 0 {
                    return Err(io::Error::last_os_error());
                }
                current = unsafe { File::from_raw_fd(fd) };
            }
        }
        Ok(current)
    }
    pub fn directory(parent: &File, value: &str) -> io::Result<File> {
        open(
            parent.as_raw_fd(),
            value,
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0,
        )
    }

    pub(super) fn directory_native(parent: &File, value: &std::ffi::OsStr) -> io::Result<File> {
        use std::os::unix::ffi::OsStrExt;
        let name = CString::new(value.as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in native name"))?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    pub(super) fn private_create_directory_native(
        parent: &File,
        value: &std::ffi::OsStr,
    ) -> io::Result<File> {
        use std::os::unix::ffi::OsStrExt;
        let name = CString::new(value.as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in native name"))?;
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            return Err(io::Error::last_os_error());
        }
        directory_native(parent, value)
    }
    pub fn mkdir(parent: &File, value: &str) -> io::Result<()> {
        mkdir_mode(parent, value, 0o700)
    }
    pub fn mkdir_mode(parent: &File, value: &str, mode: u32) -> io::Result<()> {
        let value = name(value)?;
        if unsafe { libc::mkdirat(parent.as_raw_fd(), value.as_ptr(), mode as libc::mode_t) } < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    pub fn file(parent: &File, value: &str, create: bool) -> io::Result<File> {
        open(
            parent.as_raw_fd(),
            value,
            libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | libc::O_CLOEXEC
                | if create {
                    libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL
                } else {
                    libc::O_RDONLY
                },
            0o600,
        )
    }
    pub fn private_directory(parent: &File, value: &str) -> io::Result<File> {
        directory(parent, value)
    }
    pub fn private_file(parent: &File, value: &str) -> io::Result<File> {
        file(parent, value, false)
    }
    pub fn identity(file: &File) -> io::Result<Identity> {
        let m = file.metadata()?;
        Ok(Identity {
            namespace: (m.dev(), m.ino(), m.mode()),
            links: m.nlink(),
            size: m.len(),
            modified: m.mtime() as i128 * 1_000_000_000 + m.mtime_nsec() as i128,
            changed: m.ctime() as i128 * 1_000_000_000 + m.ctime_nsec() as i128,
        })
    }
    pub fn rename(parent: &File, from: &str, to: &str, absent: bool) -> io::Result<()> {
        let from = name(from)?;
        let to = name(to)?;
        let fd = parent.as_raw_fd();
        let result = if absent {
            #[cfg(target_os = "linux")]
            {
                unsafe {
                    libc::renameat2(fd, from.as_ptr(), fd, to.as_ptr(), libc::RENAME_NOREPLACE)
                }
            }
            #[cfg(target_os = "macos")]
            {
                unsafe { libc::renameatx_np(fd, from.as_ptr(), fd, to.as_ptr(), 0x4) }
            }
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Atomic no-clobber publication unavailable",
                ));
            }
        } else {
            unsafe { libc::renameat(fd, from.as_ptr(), fd, to.as_ptr()) }
        };
        if result < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    pub fn unlink(parent: &File, value: &str) -> io::Result<()> {
        let value = name(value)?;
        if unsafe { libc::unlinkat(parent.as_raw_fd(), value.as_ptr(), 0) } < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

#[cfg(windows)]
mod native {
    use super::{File, Identity, Path};
    use std::{
        ffi::c_void,
        io,
        os::windows::{
            ffi::OsStrExt,
            io::{AsRawHandle, FromRawHandle},
        },
    };
    type Handle = *mut c_void;
    #[repr(C)]
    struct Unicode {
        length: u16,
        maximum: u16,
        buffer: *mut u16,
    }
    #[repr(C)]
    struct Attributes {
        length: u32,
        root: Handle,
        name: *mut Unicode,
        flags: u32,
        security: *mut c_void,
        quality: *mut c_void,
    }
    #[repr(C)]
    struct Status {
        status: usize,
        information: usize,
    }
    #[repr(C)]
    struct Basic {
        created: i64,
        accessed: i64,
        written: i64,
        changed: i64,
        attributes: u32,
    }
    #[repr(C)]
    struct Standard {
        allocation: i64,
        end: i64,
        links: u32,
        delete: u8,
        directory: u8,
    }
    #[repr(C)]
    struct AttributeTag {
        attributes: u32,
        tag: u32,
    }
    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[repr(C)]
    struct HandleInfo {
        attributes: u32,
        created: FileTime,
        accessed: FileTime,
        written: FileTime,
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    // Windows SDK BY_HANDLE_FILE_INFORMATION: DWORD fields and three FILETIMEs.
    const _: () = {
        assert!(std::mem::size_of::<HandleInfo>() == 52);
        assert!(std::mem::align_of::<HandleInfo>() == 4);
        assert!(std::mem::offset_of!(HandleInfo, links) == 40);
    };
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtCreateFile(
            out: *mut Handle,
            access: u32,
            attributes: *mut Attributes,
            status: *mut Status,
            size: *mut i64,
            flags: u32,
            share: u32,
            disposition: u32,
            options: u32,
            ea: *mut c_void,
            ea_len: u32,
        ) -> i32;
        fn NtQueryInformationFile(
            file: Handle,
            status: *mut Status,
            out: *mut c_void,
            size: u32,
            class: u32,
        ) -> i32;
        fn NtSetInformationFile(
            file: Handle,
            status: *mut Status,
            input: *mut c_void,
            size: u32,
            class: u32,
        ) -> i32;
        fn RtlNtStatusToDosError(status: i32) -> u32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(file: Handle, information: *mut c_void) -> i32;
    }
    fn admitted(status: i32) -> io::Result<()> {
        if status < 0 {
            Err(io::Error::from_raw_os_error(
                unsafe { RtlNtStatusToDosError(status) } as i32,
            ))
        } else {
            Ok(())
        }
    }
    fn open(
        parent: Option<&File>,
        name: &std::ffi::OsStr,
        directory: bool,
        create: bool,
        delete: bool,
    ) -> io::Result<File> {
        open_access(parent, name, directory, create, delete, false)
    }
    fn open_access(
        parent: Option<&File>,
        name: &std::ffi::OsStr,
        directory: bool,
        create: bool,
        delete: bool,
        private_acl: bool,
    ) -> io::Result<File> {
        open_security(
            parent,
            name,
            directory,
            create,
            delete,
            private_acl,
            std::ptr::null_mut(),
        )
    }
    fn open_security(
        parent: Option<&File>,
        name: &std::ffi::OsStr,
        directory: bool,
        create: bool,
        delete: bool,
        private_acl: bool,
        descriptor: *mut c_void,
    ) -> io::Result<File> {
        let mut wide = name.encode_wide().collect::<Vec<_>>();
        if wide.contains(&0) || wide.len() > 32760 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid rooted name",
            ));
        }
        let mut unicode = Unicode {
            length: (wide.len() * 2) as u16,
            maximum: (wide.len() * 2) as u16,
            buffer: wide.as_mut_ptr(),
        };
        let mut attributes = Attributes {
            length: std::mem::size_of::<Attributes>() as u32,
            root: parent
                .map(|file| file.as_raw_handle())
                .unwrap_or(std::ptr::null_mut()),
            name: &mut unicode,
            flags: 0x40,
            security: descriptor,
            quality: std::ptr::null_mut(),
        };
        let mut status = Status {
            status: 0,
            information: 0,
        };
        let mut handle = std::ptr::null_mut();
        let access = 0x00100000
            | 0x00000080
            | if directory {
                0x00020000 | 0x00000001 | 0x00000020
            } else if create {
                0x40000000
            } else {
                0x80000000
            }
            | if create && directory && private_acl {
                0x00000100
            } else {
                0
            }
            | if delete { 0x00010000 } else { 0 }
            | if private_acl {
                0x00020000 | 0x00040000 | 0x00080000
            } else {
                0
            };
        let code = unsafe {
            NtCreateFile(
                &mut handle,
                access,
                &mut attributes,
                &mut status,
                std::ptr::null_mut(),
                0,
                7,
                if create { 2 } else { 1 },
                0x00200000 | 0x20 | if directory { 1 } else { 0x40 },
                std::ptr::null_mut(),
                0,
            )
        };
        // STATUS_OBJECT_NAME_INVALID surfaces as os error 123 without the
        // name; a debug build says which one NT refused.
        if cfg!(debug_assertions) && code as u32 == 0xC000_0033 {
            eprintln!(
                "infra-core: NT refused the {} name {name:?}",
                if parent.is_some() {
                    "relative"
                } else {
                    "rooted"
                }
            );
        }
        admitted(code)?;
        let file = unsafe { File::from_raw_handle(handle) };
        let mut basic = std::mem::MaybeUninit::<Basic>::uninit();
        admitted(unsafe {
            NtQueryInformationFile(
                handle,
                &mut status,
                basic.as_mut_ptr().cast(),
                std::mem::size_of::<Basic>() as u32,
                4,
            )
        })?;
        if unsafe { basic.assume_init() }.attributes & 0x400 != 0 {
            let mut tag = std::mem::MaybeUninit::<AttributeTag>::uninit();
            admitted(unsafe {
                NtQueryInformationFile(
                    handle,
                    &mut status,
                    tag.as_mut_ptr().cast(),
                    std::mem::size_of::<AttributeTag>() as u32,
                    35,
                )
            })?;
            let tag = unsafe { tag.assume_init() }.tag;
            if tag == 0 || tag & 0x20000000 != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Redirecting reparse publication path refused",
                ));
            }
        }
        Ok(file)
    }
    pub fn root(path: &Path) -> io::Result<File> {
        use std::{
            ffi::OsString,
            path::{Component, Prefix},
        };
        let invalid = || {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Use an absolute drive or UNC publication root",
            )
        };
        let mut components = path.components();
        let Some(Component::Prefix(prefix)) = components.next() else {
            return Err(invalid());
        };
        let unc_volume = matches!(prefix.kind(), Prefix::UNC(..) | Prefix::VerbatimUNC(..));
        let mut volume = OsString::from("\\??\\");
        match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                volume.push(format!("{}:\\", drive as char));
            }
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                if server.is_empty() || share.is_empty() {
                    return Err(invalid());
                }
                volume.push("UNC\\");
                volume.push(server);
                volume.push("\\");
                volume.push(share);
                volume.push("\\");
            }
            _ => return Err(invalid()),
        }
        match components.next() {
            Some(Component::RootDir) => {}
            // Verbatim UNC share roots have no synthetic RootDir component
            // when their spelling ends immediately after the share name.
            None if unc_volume => {}
            _ => return Err(invalid()),
        }
        let children = components
            .map(|part| match part {
                Component::Normal(name) => Ok(name),
                _ => Err(invalid()),
            })
            .collect::<io::Result<Vec<_>>>()?;
        // Only the drive/share volume is named absolutely. Every user-selected
        // component is opened relative to the preceding retained directory.
        let mut directory = open(None, &volume, true, false, false)?;
        for name in children {
            directory = open(Some(&directory), name, true, false, false)?;
        }
        Ok(directory)
    }
    pub fn directory(parent: &File, value: &str) -> io::Result<File> {
        open(
            Some(parent),
            std::ffi::OsStr::new(value),
            true,
            false,
            false,
        )
    }
    pub fn mkdir(parent: &File, value: &str) -> io::Result<()> {
        open(Some(parent), std::ffi::OsStr::new(value), true, true, false).map(drop)
    }
    pub fn mkdir_mode(parent: &File, value: &str, _mode: u32) -> io::Result<()> {
        mkdir(parent, value)
    }
    pub fn file(parent: &File, value: &str, create: bool) -> io::Result<File> {
        open(
            Some(parent),
            std::ffi::OsStr::new(value),
            false,
            create,
            create,
        )
    }

    pub(super) fn directory_native(parent: &File, value: &std::ffi::OsStr) -> io::Result<File> {
        open(Some(parent), value, true, false, false)
    }
    pub(super) fn private_create_directory_native(
        parent: &File,
        value: &std::ffi::OsStr,
    ) -> io::Result<File> {
        crate::security::with_private_creation_descriptor(|descriptor| {
            open_security(Some(parent), value, true, true, false, true, descriptor)
        })
    }
    pub fn private_create_directory(parent: &File, value: &str) -> io::Result<File> {
        crate::security::with_private_creation_descriptor(|descriptor| {
            open_security(
                Some(parent),
                std::ffi::OsStr::new(value),
                true,
                true,
                false,
                true,
                descriptor,
            )
        })
    }
    pub fn private_directory(parent: &File, value: &str) -> io::Result<File> {
        open_access(
            Some(parent),
            std::ffi::OsStr::new(value),
            true,
            false,
            false,
            true,
        )
    }
    pub fn private_file(parent: &File, value: &str) -> io::Result<File> {
        open_access(
            Some(parent),
            std::ffi::OsStr::new(value),
            false,
            false,
            false,
            true,
        )
    }
    pub fn identity(file: &File) -> io::Result<Identity> {
        let mut status = Status {
            status: 0,
            information: 0,
        };
        let mut basic = std::mem::MaybeUninit::<Basic>::uninit();
        let mut standard = std::mem::MaybeUninit::<Standard>::uninit();
        let mut identifier = 0u64;
        admitted(unsafe {
            NtQueryInformationFile(
                file.as_raw_handle(),
                &mut status,
                basic.as_mut_ptr().cast(),
                std::mem::size_of::<Basic>() as u32,
                4,
            )
        })?;
        admitted(unsafe {
            NtQueryInformationFile(
                file.as_raw_handle(),
                &mut status,
                standard.as_mut_ptr().cast(),
                std::mem::size_of::<Standard>() as u32,
                5,
            )
        })?;
        admitted(unsafe {
            NtQueryInformationFile(
                file.as_raw_handle(),
                &mut status,
                (&mut identifier as *mut u64).cast(),
                8,
                6,
            )
        })?;
        let basic = unsafe { basic.assume_init() };
        let standard = unsafe { standard.assume_init() };
        let mut information = std::mem::MaybeUninit::<HandleInfo>::uninit();
        if unsafe {
            GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr().cast())
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let information = unsafe { information.assume_init() };
        let mut mode = if basic.attributes & 16 != 0 {
            0o40700
        } else {
            0o100600
        };
        if basic.attributes & 1 != 0 {
            mode &= !0o222;
        }
        Ok(Identity {
            namespace: (information.volume as u64, identifier, mode),
            links: standard.links as u64,
            size: standard.end as u64,
            modified: basic.written as i128,
            changed: basic.changed as i128,
        })
    }
    pub fn rename(parent: &File, from: &str, to: &str, absent: bool) -> io::Result<()> {
        let source = open(Some(parent), std::ffi::OsStr::new(from), false, false, true)?;
        let name = to.encode_utf16().collect::<Vec<_>>();
        let pointer = std::mem::size_of::<usize>();
        let root_offset = pointer;
        let length_offset = root_offset + pointer;
        let name_offset = length_offset + 4;
        let mut data = vec![0u8; name_offset + name.len() * 2];
        data[0] = u8::from(!absent);
        let handle = parent.as_raw_handle() as usize;
        data[root_offset..root_offset + pointer].copy_from_slice(&handle.to_ne_bytes());
        data[length_offset..length_offset + 4]
            .copy_from_slice(&(name.len() as u32 * 2).to_ne_bytes());
        for (i, unit) in name.iter().enumerate() {
            data[name_offset + 2 * i..name_offset + 2 * i + 2].copy_from_slice(&unit.to_ne_bytes());
        }
        let mut status = Status {
            status: 0,
            information: 0,
        };
        admitted(unsafe {
            NtSetInformationFile(
                source.as_raw_handle(),
                &mut status,
                data.as_mut_ptr().cast(),
                data.len() as u32,
                10,
            )
        })
    }
    pub fn unlink(parent: &File, value: &str) -> io::Result<()> {
        let source = open(
            Some(parent),
            std::ffi::OsStr::new(value),
            false,
            false,
            true,
        )?;
        let mut deletion = 1u8;
        let mut status = Status {
            status: 0,
            information: 0,
        };
        admitted(unsafe {
            NtSetInformationFile(
                source.as_raw_handle(),
                &mut status,
                (&mut deletion as *mut u8).cast(),
                1,
                13,
            )
        })
    }
}

impl PublicationFiles {
    pub fn capture_source(
        &mut self,
        relative: &str,
        limit: usize,
    ) -> Result<Option<CapturedFileImage>> {
        if limit > 512 * 1024 * 1024 {
            return Err(RuntimeError::invalid("Source capture limit exceeded"));
        }
        let (parent, name) = match self.parent(relative, false) {
            Ok(value) => value,
            Err(error) if error.kind == crate::ErrorKind::NotFound => {
                self.check()?;
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let mut file = match native::file(&parent, &name, false) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.check()?;
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        let identity = native::identity(&file)?;
        // Explicit release sources may be hardlinked by Cargo. Capture is
        // read-only; authority mutation APIs retain their one-link admission.
        if !file.metadata()?.is_file() || identity.links == 0 || identity.size > limit as u64 {
            return Err(RuntimeError::policy(
                "Release source is not an ordinary bounded file",
            ));
        }
        self.check()?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > limit
            || native::identity(&file)? != identity
            || native::identity(&native::file(&parent, &name, false)?)? != identity
        {
            return Err(RuntimeError::conflict(
                "Release source changed during capture",
            ));
        }
        self.check()?;
        Ok(Some((
            bytes,
            (
                identity.namespace.0,
                identity.namespace.1,
                identity.namespace.2,
                identity.links,
                identity.size,
                identity.modified,
                identity.changed,
            ),
        )))
    }
}

#[cfg(test)]
mod kept_mode_tests {
    use super::{PublicationFiles, kept_mode};

    #[test]
    fn a_published_mode_reads_back_as_kept_mode_says() {
        let root = std::env::temp_dir().join(format!("infra-kept-mode-{}", crate::domain::uuid4()));
        std::fs::create_dir(&root).unwrap();
        let mut files = PublicationFiles::new(&root).unwrap();
        let mut observed = vec![];
        for mode in [0o755, 0o644, 0o600, 0o555, 0o444] {
            let name = format!("file-{mode:o}");
            files
                .write_expected_mode(&name, b"bytes", None, mode, 64)
                .unwrap();
            observed.push((mode, files.read_mode(&name).unwrap()));
        }
        drop(files);
        let _ = std::fs::remove_dir_all(&root);
        for (mode, read) in observed {
            assert_eq!(read, Some(kept_mode(mode)), "{mode:o}");
        }
    }
}
