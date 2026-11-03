//! Reserved Cursor rule custody. All effect paths are relative to held native parents.
//! This deliberately keeps compatibility receipt clocks separate from stronger CAS clocks.
use super::{Identity, Pin, PublicationFiles, native};
use crate::{Result, RuntimeError};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::Path,
};

#[derive(Clone, Debug, PartialEq)]
pub struct CursorImage {
    pub bytes: Vec<u8>,
    pub receipt_identity: Value,
    strong: Identity,
}
pub struct CursorFiles {
    files: PublicationFiles,
}

impl CursorFiles {
    pub fn new(root: &Path) -> Result<Self> {
        let root = crate::fs::absolute(root)?;
        let file = cursor_native::root(&root)?;
        let identity = native::identity(&file)?;
        let mut directories = BTreeMap::new();
        directories.insert(vec![], file);
        let result = Self {
            files: PublicationFiles {
                root,
                root_identity: identity.namespace,
                directories,
                pins: vec![],
            },
        };
        result.check()?;
        Ok(result)
    }
    pub fn root_identity(&self) -> [u64; 2] {
        [self.files.root_identity.0, self.files.root_identity.1]
    }
    pub fn check(&self) -> Result<()> {
        if {
            let a = native::identity(&cursor_native::root(&self.files.root)?)?.namespace;
            (a.0, a.1)
        } != (self.files.root_identity.0, self.files.root_identity.1)
        {
            return Err(RuntimeError::conflict(
                "Cursor publication root custody changed",
            ));
        }
        for pin in &self.files.pins {
            let named = cursor_native::directory(&pin.parent, &pin.name)?;
            let a = native::identity(&named)?.namespace;
            let b = native::identity(&pin.child)?.namespace;
            if (a.0, a.1) != (pin.identity.0, pin.identity.1)
                || (b.0, b.1) != (pin.identity.0, pin.identity.1)
            {
                return Err(RuntimeError::conflict(
                    "Cursor publication directory custody changed",
                ));
            }
        }
        Ok(())
    }
    fn directory_parts(&mut self, parts: &[&str], create: bool) -> Result<Option<File>> {
        for count in 1..=parts.len() {
            let key = parts[..count]
                .iter()
                .map(|part| part.to_string())
                .collect::<Vec<_>>();
            if self.files.directories.contains_key(&key) {
                continue;
            }
            let previous = key[..key.len() - 1].to_vec();
            let Some(parent) = self.files.directories.get(&previous) else {
                return Ok(None);
            };
            let parent = parent.try_clone()?;
            let child = match cursor_native::directory(&parent, parts[count - 1]) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && !create => {
                    self.check()?;
                    return Ok(None);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
                    self.check()?;
                    match cursor_native::mkdir(&parent, parts[count - 1]) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(error) => return Err(error.into()),
                    }
                    cursor_native::directory(&parent, parts[count - 1])?
                }
                Err(error) => return Err(error.into()),
            };
            let identity = native::identity(&child)?.namespace;
            self.files.pins.push(Pin {
                parent,
                name: parts[count - 1].into(),
                child: child.try_clone()?,
                identity,
            });
            self.files.directories.insert(key, child);
        }
        self.check()?;
        let key = parts
            .iter()
            .map(|part| part.to_string())
            .collect::<Vec<_>>();
        self.files
            .directories
            .get(&key)
            .map(File::try_clone)
            .transpose()
            .map_err(Into::into)
    }
    fn parts(relative: &str) -> Result<Vec<&str>> {
        let parts = relative.split('/').collect::<Vec<_>>();
        if relative.starts_with('/')
            || relative.contains(['\\', '\0'])
            || parts
                .iter()
                .any(|part| part.is_empty() || *part == "." || *part == "..")
        {
            return Err(RuntimeError::policy("Invalid Cursor publication path"));
        }
        #[cfg(windows)]
        for part in &parts {
            cursor_native::component(part)?;
        }
        Ok(parts)
    }
    fn parent(&mut self, relative: &str, create: bool) -> Result<Option<(File, String)>> {
        let parts = Self::parts(relative)?;
        Ok(self
            .directory_parts(&parts[..parts.len() - 1], create)?
            .map(|parent| (parent, parts[parts.len() - 1].into())))
    }
    pub fn directory_identity(&mut self, relative: &str, create: bool) -> Result<Option<[u64; 2]>> {
        let parts = Self::parts(relative)?;
        self.directory_parts(&parts, create)?
            .map(|file| {
                native::identity(&file)
                    .map(|i| [i.namespace.0, i.namespace.1])
                    .map_err(Into::into)
            })
            .transpose()
    }
    pub fn image(
        &mut self,
        relative: &str,
        limit: usize,
        links: u64,
    ) -> Result<Option<CursorImage>> {
        if ![1, 2].contains(&links) {
            return Err(RuntimeError::policy("Invalid Cursor link admission"));
        }
        let Some((parent, name)) = self.parent(relative, false)? else {
            return Ok(None);
        };
        let mut file = match cursor_native::file(&parent, &name, false) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.check()?;
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        let identity = native::identity(&file)?;
        if !file.metadata()?.is_file() || identity.links != links || identity.size > limit as u64 {
            return Err(RuntimeError::policy("Unsupported Cursor file custody"));
        }
        self.check()?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)?;
        let named = cursor_native::file(&parent, &name, false)?;
        if bytes.len() > limit
            || bytes.len() as u64 != identity.size
            || native::identity(&file)? != identity
            || native::identity(&named)? != identity
        {
            return Err(RuntimeError::conflict("Cursor file custody changed"));
        }
        std::str::from_utf8(&bytes)
            .map_err(|_| RuntimeError::invalid("Cursor file is not UTF-8"))?;
        let receipt_identity = cursor_native::receipt_identity(&file, &identity)?;
        if native::identity(&file)? != identity {
            return Err(RuntimeError::conflict(
                "Cursor file changed during compatibility metadata read",
            ));
        }
        self.check()?;
        Ok(Some(CursorImage {
            bytes,
            receipt_identity,
            strong: identity,
        }))
    }
    pub fn publish(
        &mut self,
        relative: &str,
        content: &[u8],
        expected: Option<&CursorImage>,
        limit: usize,
    ) -> Result<()> {
        if content.len() > limit {
            return Err(RuntimeError::policy("Cursor publication admission bound"));
        }
        if self.image(relative, limit, 1)?.as_ref() != expected {
            return Err(RuntimeError::conflict("Cursor publication image changed"));
        }
        let (parent, name) = self
            .parent(relative, true)?
            .ok_or_else(|| RuntimeError::unavailable("Cursor publication parent unavailable"))?;
        let temporary = format!(".cursor-rule-{}.tmp", uuid::Uuid::new_v4());
        let mut file = cursor_native::file(&parent, &temporary, true)?;
        let owned = native::identity(&file)?.namespace;
        let result = (|| -> Result<()> {
            file.write_all(content)?;
            file.flush()?;
            drop(file);
            self.check()?;
            if self.image(relative, limit, 1)?.as_ref() != expected {
                return Err(RuntimeError::conflict(
                    "Cursor publication image changed before commit",
                ));
            }
            cursor_native::rename(&parent, &temporary, &name, expected.is_none())?;
            self.check()?;
            Ok(())
        })();
        if cursor_native::file(&parent, &temporary, false)
            .ok()
            .and_then(|file| native::identity(&file).ok())
            .is_some_and(|current| current.namespace == owned)
        {
            let cleanup = cursor_native::unlink(&parent, &temporary);
            if result.is_ok() {
                cleanup?;
            }
        }
        result
    }
    pub fn unlink(&mut self, relative: &str, expected: &CursorImage, limit: usize) -> Result<()> {
        if self.image(relative, limit, 1)?.as_ref() != Some(expected) {
            return Err(RuntimeError::conflict("Cursor removal image changed"));
        }
        let (parent, name) = self
            .parent(relative, false)?
            .ok_or_else(|| RuntimeError::conflict("Cursor removal parent disappeared"))?;
        self.check()?;
        cursor_native::unlink(&parent, &name)?;
        Ok(())
    }
    pub fn lock(&mut self) -> Result<crate::fs::MutationLock> {
        let (parent, name) = self
            .parent("project-brain/local/runtime.lock", true)?
            .ok_or_else(|| RuntimeError::unavailable("Cursor lock parent unavailable"))?;
        self.check()?;
        let file = cursor_native::lock_file(&parent, &name)?;
        let identity = native::identity(&file)?;
        let named = cursor_native::file(&parent, &name, false)?;
        if !file.metadata()?.is_file()
            || identity.links != 1
            || native::identity(&named)? != identity
        {
            return Err(RuntimeError::policy("Unsupported Cursor lock custody"));
        }
        self.check()?;
        crate::fs::MutationLock::try_acquire_anchored(&self.files.root, file)
    }
    /// Only Unix's old link-based producer has the recoverable two-name window.
    pub fn legacy_alias(
        &mut self,
        relative: &str,
        current: &CursorImage,
    ) -> Result<Option<String>> {
        if !cfg!(unix) || current.strong.links != 2 {
            return Ok(None);
        }
        let Some((parent, _)) = self.parent(relative, false)? else {
            return Ok(None);
        };
        let mut alias = None;
        let names = cursor_native::names(&parent, Some(1024))?;
        for name in names {
            if !temporary_name(&name, true) {
                continue;
            }
            if !cursor_native::regular_entry(&parent, &name)? {
                continue;
            }
            let opened = match cursor_native::file(&parent, &name, false) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            let image = native::identity(&opened)?;
            if (image.namespace.0, image.namespace.1)
                == (current.strong.namespace.0, current.strong.namespace.1)
            {
                if alias.is_some() || !opened.metadata()?.is_file() || image.links != 2 {
                    return Ok(None);
                }
                alias = Some(name);
            }
        }
        self.check()?;
        Ok(alias.map(|name| match relative.rsplit_once('/') {
            Some((parent, _)) => format!("{parent}/{name}"),
            None => name,
        }))
    }
    pub fn reconcile_alias(
        &mut self,
        relative: &str,
        alias: &str,
        current: &CursorImage,
        limit: usize,
    ) -> Result<bool> {
        if !cfg!(unix)
            || self.image(relative, limit, 2)?.as_ref() != Some(current)
            || self.image(alias, limit, 2)?.as_ref() != Some(current)
        {
            return Ok(false);
        }
        let (parent, name) = self
            .parent(alias, false)?
            .ok_or_else(|| RuntimeError::conflict("Cursor alias parent disappeared"))?;
        self.check()?;
        cursor_native::unlink(&parent, &name)?;
        let observed = self.image(relative, limit, 1)?;
        self.check()?;
        Ok(observed.is_some_and(|image| {
            image.bytes == current.bytes
                && image
                    .receipt_identity
                    .as_array()
                    .zip(current.receipt_identity.as_array())
                    .is_some_and(|(a, b)| a[..4] == b[..4])
        }))
    }
    pub fn sweep_temporaries(&mut self, relative: &str) -> Result<()> {
        if !cfg!(unix) {
            return Ok(());
        }
        let parts = Self::parts(relative)?;
        let Some(parent) = self.directory_parts(&parts, false)? else {
            return Ok(());
        };
        let cutoff = crate::domain::clock::proof_now()
            .timestamp_nanos_opt()
            .map(i128::from)
            .unwrap_or(i128::MAX)
            - 60_000_000_000;
        for name in cursor_native::names(&parent, None)? {
            if !temporary_name(&name, false) {
                continue;
            }
            if !cursor_native::regular_entry(&parent, &name)? {
                continue;
            }
            let file = match cursor_native::file(&parent, &name, false) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            let identity = native::identity(&file)?;
            if file.metadata()?.is_file() && identity.modified < cutoff {
                self.check()?;
                if cursor_native::file(&parent, &name, false)
                    .ok()
                    .and_then(|file| native::identity(&file).ok())
                    .as_ref()
                    == Some(&identity)
                {
                    match cursor_native::unlink(&parent, &name) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error.into()),
                    }
                }
            }
        }
        self.check()
    }
    pub fn preserve_edit(&mut self, raw: &[u8]) -> Result<String> {
        let name = format!(
            "{}-{}.mdc",
            crate::domain::clock::proof_now().format("%Y%m%dT%H%M%SZ"),
            &super::sha256(raw)[..12]
        );
        let relative = format!("memory-bank/local/cursor-rule-edits/{name}");
        match self.image(&relative, 16384, 1)? {
            None => self.publish(&relative, raw, None, 16384)?,
            Some(image) if image.bytes == raw => {}
            Some(_) => return Err(RuntimeError::conflict("Cursor edit copy custody changed")),
        }
        if cfg!(unix) {
            let parts = Self::parts("memory-bank/local/cursor-rule-edits")?;
            let parent = self
                .directory_parts(&parts, false)?
                .ok_or_else(|| RuntimeError::unavailable("Cursor edit directory unavailable"))?;
            let mut names = cursor_native::names(&parent, None)?
                .into_iter()
                .filter(|name| edit_name(name))
                .collect::<Vec<_>>();
            names.sort();
            let remove = names.len().saturating_sub(20);
            for name in names.into_iter().take(remove) {
                if !cursor_native::regular_entry(&parent, &name)? {
                    continue;
                }
                let file = match cursor_native::file(&parent, &name, false) {
                    Ok(file) => file,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error.into()),
                };
                let identity = native::identity(&file)?;
                if file.metadata()?.is_file() {
                    self.check()?;
                    if cursor_native::file(&parent, &name, false)
                        .ok()
                        .and_then(|file| native::identity(&file).ok())
                        .as_ref()
                        == Some(&identity)
                    {
                        match cursor_native::unlink(&parent, &name) {
                            Ok(()) => {}
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                            Err(error) => return Err(error.into()),
                        }
                    }
                }
            }
        }
        self.check()?;
        Ok(relative)
    }
}
/// Shared anchored enumeration for other publication extensions. Invalid Unicode
/// refuses the operation instead of hiding a child from an archive plan.
pub(super) fn rooted_lock_file(parent: &File, name: &str) -> std::io::Result<File> {
    cursor_native::lock_file(parent, name)
}

pub(crate) fn cursor_names(parent: &File, limit: Option<usize>) -> Result<Vec<String>> {
    cursor_native::names_strict(parent, limit)
}
fn temporary_name(name: &str, strict: bool) -> bool {
    let Some(inner) = name
        .strip_prefix(".cursor-rule-")
        .and_then(|name| name.strip_suffix(".tmp"))
    else {
        return false;
    };
    if !strict {
        return inner.len() == 36
            && inner.bytes().all(|byte| {
                byte.is_ascii_hexdigit() && !(b'A'..=b'F').contains(&byte) || byte == b'-'
            });
    }
    uuid::Uuid::parse_str(inner).is_ok_and(|id| {
        id.get_version_num() == 4
            && id.get_variant() == uuid::Variant::RFC4122
            && id.to_string() == inner
    })
}
fn edit_name(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 33
        && b[..8].iter().all(u8::is_ascii_digit)
        && b[8] == b'T'
        && b[9..15].iter().all(u8::is_ascii_digit)
        && &b[15..17] == b"Z-"
        && b[17..29]
            .iter()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(v))
        && &b[29..] == b".mdc"
}

#[cfg(unix)]
mod cursor_native {
    use super::*;
    use std::{
        ffi::{CStr, CString},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::MetadataExt,
        },
    };
    pub fn root(path: &Path) -> std::io::Result<File> {
        native::root(path)
    }
    pub fn directory(parent: &File, name: &str) -> std::io::Result<File> {
        native::directory(parent, name)
    }
    pub fn mkdir(parent: &File, name: &str) -> std::io::Result<()> {
        native::mkdir(parent, name)
    }
    pub fn file(parent: &File, name: &str, create: bool) -> std::io::Result<File> {
        native::file(parent, name, create)
    }
    pub fn unlink(parent: &File, name: &str) -> std::io::Result<()> {
        native::unlink(parent, name)
    }
    pub fn regular_entry(parent: &File, name: &str) -> std::io::Result<bool> {
        let name = CString::new(name).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Invalid Cursor entry name",
            )
        })?;
        let mut image = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                name.as_ptr(),
                image.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } < 0
        {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(false);
            }
            return Err(error);
        }
        Ok(unsafe { image.assume_init() }.st_mode & libc::S_IFMT == libc::S_IFREG)
    }
    pub fn lock_file(parent: &File, name: &str) -> std::io::Result<File> {
        let name = CString::new(name).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid lock name")
        })?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDWR
                    | libc::O_CREAT
                    | libc::O_NONBLOCK
                    | libc::O_NOFOLLOW
                    | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    pub fn receipt_identity(file: &File, identity: &Identity) -> Result<Value> {
        let m = file.metadata()?;
        let modified = m.mtime() as i128 * 1_000_000_000 + m.mtime_nsec() as i128;
        let changed = m.ctime() as i128 * 1_000_000_000 + m.ctime_nsec() as i128;
        crate::domain::json::parse(&format!(
            "[{},{},{},{modified},{changed}]",
            identity.namespace.0, identity.namespace.1, identity.size
        ))
    }
    pub fn rename(parent: &File, from: &str, to: &str, absent: bool) -> std::io::Result<()> {
        if !absent {
            return native::rename(parent, from, to, false);
        }
        match native::rename(parent, from, to, true) {
            Ok(()) => return Ok(()),
            Err(error)
                if [
                    Some(libc::ENOSYS),
                    Some(libc::EINVAL),
                    Some(libc::EOPNOTSUPP),
                ]
                .contains(&error.raw_os_error())
                    || error.kind() == std::io::ErrorKind::Unsupported => {}
            Err(error) => return Err(error),
        }
        let from = CString::new(from).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid publication name")
        })?;
        let to = CString::new(to).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid publication name")
        })?;
        if unsafe {
            libc::linkat(
                parent.as_raw_fd(),
                from.as_ptr(),
                parent.as_raw_fd(),
                to.as_ptr(),
                0,
            )
        } < 0
        {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    pub fn names(parent: &File, limit: Option<usize>) -> Result<Vec<String>> {
        names_impl(parent, limit, false)
    }
    pub fn names_strict(parent: &File, limit: Option<usize>) -> Result<Vec<String>> {
        names_impl(parent, limit, true)
    }
    fn names_impl(parent: &File, limit: Option<usize>, strict: bool) -> Result<Vec<String>> {
        let dot = CString::new(".").expect("fixed directory name");
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                dot.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let directory = unsafe { libc::fdopendir(fd) };
        if directory.is_null() {
            unsafe { libc::close(fd) };
            return Err(std::io::Error::last_os_error().into());
        }
        struct Close(*mut libc::DIR);
        impl Drop for Close {
            fn drop(&mut self) {
                unsafe { libc::closedir(self.0) };
            }
        }
        let _close = Close(directory);
        let mut names = vec![];
        let mut count = 0usize;
        loop {
            let entry = unsafe { libc::readdir(directory) };
            if entry.is_null() {
                break;
            }
            let raw = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if raw == b"." || raw == b".." {
                continue;
            }
            count += 1;
            if limit.is_some_and(|limit| count > limit) {
                return Err(RuntimeError::policy(
                    "Cursor legacy alias directory bound exceeded",
                ));
            }
            match std::str::from_utf8(raw) {
                Ok(name) => names.push(name.into()),
                Err(_) if strict => {
                    return Err(RuntimeError::invalid(
                        "Publication directory name is not UTF-8",
                    ));
                }
                Err(_) => {}
            }
        }
        Ok(names)
    }
}

#[cfg(windows)]
mod cursor_native {
    use super::*;
    use std::{
        ffi::{OsStr, c_void},
        io,
        os::windows::{
            ffi::OsStrExt,
            fs::MetadataExt,
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
    // IO_STATUS_BLOCK starts with a pointer-sized status/pointer union.
    const _: () = {
        assert!(std::mem::size_of::<Status>() == 2 * std::mem::size_of::<usize>());
        assert!(std::mem::align_of::<Status>() == std::mem::align_of::<usize>());
        assert!(std::mem::offset_of!(Status, information) == std::mem::size_of::<usize>());
    };
    #[repr(C)]
    struct AttributeTag {
        attributes: u32,
        tag: u32,
    }
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
        fn NtSetInformationFile(
            file: Handle,
            status: *mut Status,
            input: *mut c_void,
            size: u32,
            class: u32,
        ) -> i32;
        fn RtlNtStatusToDosError(status: i32) -> u32;
        fn NtQueryDirectoryFile(
            file: Handle,
            event: Handle,
            apc: *mut c_void,
            apc_context: *mut c_void,
            status: *mut Status,
            information: *mut c_void,
            length: u32,
            class: i32,
            single: u8,
            name: *const c_void,
            restart: u8,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *const c_void,
            disposition: u32,
            flags: u32,
            template: Handle,
        ) -> Handle;
        fn GetFileInformationByHandleEx(
            file: Handle,
            class: i32,
            information: *mut c_void,
            size: u32,
        ) -> i32;
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
    pub fn component(name: &str) -> io::Result<()> {
        let unsafe_character = name
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':' | '\0') || c < ' ');
        let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
        let device = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
            || stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3] >= b'1'
                && stem.as_bytes()[3] <= b'9';
        if name.is_empty()
            || [".", ".."].contains(&name)
            || name.ends_with([' ', '.'])
            || unsafe_character
            || device
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Unsafe Windows Cursor component",
            ));
        }
        Ok(())
    }
    fn metadata(file: &File) -> io::Result<()> {
        let mut tag = std::mem::MaybeUninit::<AttributeTag>::uninit();
        if unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                9,
                tag.as_mut_ptr().cast(),
                std::mem::size_of::<AttributeTag>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if unsafe { tag.assume_init() }.attributes & 0x400 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Cursor reparse point refused",
            ));
        }
        Ok(())
    }
    fn open(
        parent: &File,
        name: &str,
        access: u32,
        disposition: u32,
        directory: bool,
    ) -> io::Result<File> {
        component(name)?;
        let mut wide = OsStr::new(name).encode_wide().collect::<Vec<_>>();
        if wide.len() > 32760 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Windows Cursor component too long",
            ));
        }
        let mut unicode = Unicode {
            length: (wide.len() * 2) as u16,
            maximum: (wide.len() * 2) as u16,
            buffer: wide.as_mut_ptr(),
        };
        let mut attributes = Attributes {
            length: std::mem::size_of::<Attributes>() as u32,
            root: parent.as_raw_handle(),
            name: &mut unicode,
            flags: 0x40,
            security: std::ptr::null_mut(),
            quality: std::ptr::null_mut(),
        };
        let mut status = Status {
            status: 0,
            information: 0,
        };
        let mut handle = std::ptr::null_mut();
        admitted(unsafe {
            NtCreateFile(
                &mut handle,
                access | 0x00100000 | 0x80,
                &mut attributes,
                &mut status,
                std::ptr::null_mut(),
                0x80,
                3,
                disposition,
                0x00200000 | 0x20 | if directory { 1 } else { 0x40 },
                std::ptr::null_mut(),
                0,
            )
        })?;
        let file = unsafe { File::from_raw_handle(handle) };
        metadata(&file)?;
        Ok(file)
    }
    pub fn root(path: &Path) -> io::Result<File> {
        use std::path::{Component, Prefix};
        let mut parts = path.components();
        let drive = match parts.next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => letter,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Use an absolute local Windows Cursor root",
                    ));
                }
            },
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Use an absolute local Windows Cursor root",
                ));
            }
        };
        if !matches!(parts.next(), Some(Component::RootDir)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Use an absolute Windows Cursor root",
            ));
        }
        let volume = format!("\\\\?\\{}:\\", drive as char)
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let handle = unsafe {
            CreateFileW(
                volume.as_ptr(),
                0x00100000 | 0x80 | 1 | 0x20,
                3,
                std::ptr::null_mut(),
                3,
                0x02000000 | 0x00200000,
                std::ptr::null_mut(),
            )
        };
        if handle.is_null() || handle as isize == -1 {
            return Err(io::Error::last_os_error());
        }
        let mut file = unsafe { File::from_raw_handle(handle) };
        metadata(&file)?;
        for part in parts {
            match part {
                Component::Normal(name) => {
                    let name = name.to_str().ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "Windows Cursor path is not Unicode",
                        )
                    })?;
                    file = directory(&file, name)?;
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Unsafe Windows Cursor root",
                    ));
                }
            }
        }
        Ok(file)
    }
    pub fn directory(parent: &File, name: &str) -> io::Result<File> {
        open(parent, name, 1 | 0x20, 1, true)
    }
    pub fn mkdir(parent: &File, name: &str) -> io::Result<()> {
        open(parent, name, 1 | 0x20, 2, true).map(drop)
    }
    pub fn file(parent: &File, name: &str, create: bool) -> io::Result<File> {
        open(
            parent,
            name,
            if create { 1 | 2 } else { 1 },
            if create { 2 } else { 1 },
            false,
        )
    }
    pub fn regular_entry(parent: &File, name: &str) -> io::Result<bool> {
        file(parent, name, false)?.metadata().map(|m| m.is_file())
    }
    pub fn lock_file(parent: &File, name: &str) -> io::Result<File> {
        open(parent, name, 1 | 2, 3, false)
    }
    fn rename_open(source: &File, parent: &File, name: &str, replace: bool) -> io::Result<()> {
        component(name)?;
        let name = name.encode_utf16().collect::<Vec<_>>();
        let pointer = std::mem::size_of::<usize>();
        let root_offset = pointer;
        let length_offset = root_offset + pointer;
        let name_offset = length_offset + 4;
        let mut data = vec![0u8; name_offset + name.len() * 2];
        data[0] = u8::from(replace);
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
    pub fn rename(parent: &File, from: &str, to: &str, absent: bool) -> io::Result<()> {
        let source = open(parent, from, 0x00010000, 1, false)?;
        rename_open(&source, parent, to, !absent)
    }
    pub fn unlink(parent: &File, name: &str) -> io::Result<()> {
        let source = open(parent, name, 0x00010000, 1, false)?;
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
    pub fn receipt_identity(file: &File, identity: &Identity) -> Result<Value> {
        let metadata = file.metadata()?;
        let time = |ticks: u64| (ticks as i128 - 116_444_736_000_000_000i128).max(0) * 100;
        let modified = time(metadata.last_write_time());
        let created = time(metadata.creation_time());
        crate::domain::json::parse(&format!(
            "[{},{},{},{modified},{created}]",
            identity.namespace.0, identity.namespace.1, identity.size
        ))
    }
    // Cursor's canonical NT profile does not call enumeration; archive publication
    // shares the same rooted primitive and supplies its own finite membership cap.
    pub fn names(parent: &File, limit: Option<usize>) -> Result<Vec<String>> {
        names_strict(parent, limit)
    }
    pub fn names_strict(parent: &File, limit: Option<usize>) -> Result<Vec<String>> {
        let mut wide = [b'.' as u16];
        let mut unicode = Unicode {
            length: 2,
            maximum: 2,
            buffer: wide.as_mut_ptr(),
        };
        let mut attributes = Attributes {
            length: std::mem::size_of::<Attributes>() as u32,
            root: parent.as_raw_handle(),
            name: &mut unicode,
            flags: 0x40,
            security: std::ptr::null_mut(),
            quality: std::ptr::null_mut(),
        };
        let mut status = Status {
            status: 0,
            information: 0,
        };
        let mut handle = std::ptr::null_mut();
        admitted(unsafe {
            NtCreateFile(
                &mut handle,
                0x00100000 | 0x80 | 1 | 0x20,
                &mut attributes,
                &mut status,
                std::ptr::null_mut(),
                0,
                3,
                1,
                0x00200000 | 0x20 | 1,
                std::ptr::null_mut(),
                0,
            )
        })?;
        let file = unsafe { File::from_raw_handle(handle) };
        metadata(&file)?;
        if native::identity(&file)?.namespace != native::identity(parent)?.namespace {
            return Err(RuntimeError::conflict(
                "Publication enumeration directory changed",
            ));
        }
        let mut buffer = vec![0u8; 65536];
        let mut first = true;
        let mut names = vec![];
        let mut count = 0usize;
        loop {
            status.information = 0;
            let result = unsafe {
                NtQueryDirectoryFile(
                    file.as_raw_handle(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut status,
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                    12,
                    0,
                    std::ptr::null_mut(),
                    u8::from(first),
                )
            };
            first = false;
            if result as u32 == 0x80000006 {
                break;
            }
            if result < 0 && result as u32 != 0x80000005 {
                admitted(result)?;
            }
            let used = status.information;
            if used < 12 || used > buffer.len() {
                return Err(RuntimeError::unavailable(
                    "Invalid native publication directory frame",
                ));
            }
            let mut at = 0usize;
            loop {
                if used - at < 12 {
                    return Err(RuntimeError::unavailable(
                        "Incomplete native publication directory record",
                    ));
                }
                let next = u32::from_le_bytes(
                    buffer[at..at + 4]
                        .try_into()
                        .map_err(|_| RuntimeError::invalid("Invalid directory offset"))?,
                ) as usize;
                let length = u32::from_le_bytes(
                    buffer[at + 8..at + 12]
                        .try_into()
                        .map_err(|_| RuntimeError::invalid("Invalid directory length"))?,
                ) as usize;
                let available = if next == 0 { used - at } else { next };
                if available < 12
                    || available > used - at
                    || next != 0 && !next.is_multiple_of(4)
                    || !length.is_multiple_of(2)
                    || length > available - 12
                {
                    return Err(RuntimeError::unavailable(
                        "Invalid native publication directory record",
                    ));
                }
                let units = buffer[at + 12..at + 12 + length]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .collect::<Vec<_>>();
                let name = String::from_utf16(&units).map_err(|_| {
                    RuntimeError::invalid("Publication directory name is not Unicode")
                })?;
                if ![".", ".."].contains(&name.as_str()) {
                    count += 1;
                    if limit.is_some_and(|limit| count > limit) {
                        return Err(RuntimeError::policy(
                            "Publication directory membership limit exceeded",
                        ));
                    }
                    component(&name)?;
                    names.push(name);
                }
                if next == 0 {
                    break;
                }
                at += next;
            }
        }
        if native::identity(&file)?.namespace != native::identity(parent)?.namespace {
            return Err(RuntimeError::conflict(
                "Publication enumeration directory changed",
            ));
        }
        Ok(names)
    }
}
#[cfg(not(any(unix, windows)))]
mod cursor_native {
    use super::*;
    fn unsupported<T>() -> std::io::Result<T> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Cursor custody unsupported",
        ))
    }
    pub fn root(_: &Path) -> std::io::Result<File> {
        unsupported()
    }
    pub fn directory(_: &File, _: &str) -> std::io::Result<File> {
        unsupported()
    }
    pub fn mkdir(_: &File, _: &str) -> std::io::Result<()> {
        unsupported()
    }
    pub fn file(_: &File, _: &str, _: bool) -> std::io::Result<File> {
        unsupported()
    }
    pub fn regular_entry(_: &File, _: &str) -> std::io::Result<bool> {
        unsupported()
    }
    pub fn lock_file(_: &File, _: &str) -> std::io::Result<File> {
        unsupported()
    }
    pub fn rename(_: &File, _: &str, _: &str, _: bool) -> std::io::Result<()> {
        unsupported()
    }
    pub fn unlink(_: &File, _: &str) -> std::io::Result<()> {
        unsupported()
    }
    pub fn receipt_identity(_: &File, _: &Identity) -> Result<Value> {
        Err(RuntimeError::unavailable("Cursor custody unsupported"))
    }
    pub fn names(_: &File, _: Option<usize>) -> Result<Vec<String>> {
        Err(RuntimeError::unavailable("Cursor custody unsupported"))
    }
    pub fn names_strict(_: &File, _: Option<usize>) -> Result<Vec<String>> {
        Err(RuntimeError::unavailable("Cursor custody unsupported"))
    }
}
