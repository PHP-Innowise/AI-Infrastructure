//! Bounded enumeration through retained directory handles. Names are discovery aids;
//! consumers must still read through PublicationFiles to bind bytes and identities.
use super::{Pin, PublicationFiles, native};
use crate::{Result, RuntimeError};
use std::{collections::BTreeSet, fs::File};
fn walk_name(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains(['/', '\\', '\0'])
        && value.chars().all(|c| c >= '\u{20}' && c != '\u{7f}')
        && (!cfg!(windows)
            || (!value.contains([':', '<', '>', '"', '|', '?', '*'])
                && !value.ends_with(['.', ' '])))
}
fn walk_ordinary(file: &File, directory: bool) -> std::io::Result<bool> {
    let meta = file.metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Ok(false);
        }
    }
    Ok(if directory {
        meta.is_dir()
    } else {
        meta.is_file()
    })
}
impl PublicationFiles {
    pub fn create_directory(&mut self, relative: &str) -> Result<()> {
        let (parent, name) = self.parent(relative, false)?;
        self.check()?;
        native::mkdir(&parent, &name)?;
        let child = native::directory(&parent, &name)?;
        let identity = native::identity(&child)?.namespace;
        self.pins.push(Pin {
            parent,
            name,
            child,
            identity,
        });
        self.check()
    }

    pub fn walk_files<F>(
        &mut self,
        max_depth: usize,
        max_nodes: usize,
        descend: F,
    ) -> Result<(Vec<String>, usize)>
    where
        F: FnMut(&str) -> bool,
    {
        self.walk_files_at("", max_depth, max_nodes, descend, false)
    }
    pub fn audit_tree(
        &mut self,
        relative: &str,
        max_depth: usize,
        max_nodes: usize,
    ) -> Result<(Vec<String>, usize)> {
        self.walk_files_at(relative, max_depth, max_nodes, |_| true, true)
    }
    /// Strict rooted admission with explicit directory exclusions. Excluded
    /// release-artifact directories must be admitted independently by callers.
    pub fn audit_tree_filtered<F>(
        &mut self,
        relative: &str,
        max_depth: usize,
        max_nodes: usize,
        descend: F,
    ) -> Result<(Vec<String>, usize)>
    where
        F: FnMut(&str) -> bool,
    {
        self.walk_files_at(relative, max_depth, max_nodes, descend, true)
    }
    fn walk_files_at<F>(
        &mut self,
        relative: &str,
        max_depth: usize,
        max_nodes: usize,
        mut descend: F,
        strict: bool,
    ) -> Result<(Vec<String>, usize)>
    where
        F: FnMut(&str) -> bool,
    {
        if max_depth > 128 || max_nodes > 1_000_000 {
            return Err(RuntimeError::invalid(
                "Rooted enumeration limits exceed bounds",
            ));
        }
        self.check()?;
        let (root, prefix) = if relative.is_empty() {
            (
                self.directories
                    .get(&vec![])
                    .ok_or_else(|| RuntimeError::unavailable("Rooted enumeration root missing"))?
                    .try_clone()?,
                vec![],
            )
        } else {
            let (parent, _) = self.parent(&format!("{relative}/.native-tree-audit"), false)?;
            (parent, relative.split('/').map(str::to_string).collect())
        };
        let mut pending = vec![(prefix, root, 0usize)];
        let mut files = vec![];
        let mut visited = 0usize;
        let mut omitted = 0usize;
        let mut seen = BTreeSet::new();
        while let Some((prefix, directory, depth)) = pending.pop() {
            if depth > max_depth || visited >= max_nodes {
                omitted += 1;
                continue;
            }
            self.check()?;
            let identity = native::identity(&directory)?.namespace;
            let (names, truncated) = walk_names(
                &directory,
                max_nodes.saturating_sub(visited).saturating_add(1),
            )?;
            if truncated {
                omitted += 1;
            }
            let mut names = names;
            names.sort();
            names.dedup();
            for name in names {
                if visited >= max_nodes {
                    omitted += 1;
                    break;
                }
                visited += 1;
                if !walk_name(&name) {
                    if strict {
                        return Err(RuntimeError::policy(
                            "Unsafe name in native runtime storage",
                        ));
                    }
                    continue;
                }
                let mut key = prefix.clone();
                key.push(name.clone());
                let relative = key.join("/");
                if let Ok(child) = native::directory(&directory, &name) {
                    if !walk_ordinary(&child, true)? {
                        if strict {
                            return Err(RuntimeError::policy(
                                "Unsafe directory in native runtime storage",
                            ));
                        }
                        continue;
                    }
                    if !descend(&relative) {
                        continue;
                    }
                    if depth >= max_depth {
                        omitted += 1;
                        continue;
                    }
                    let child_identity = native::identity(&child)?.namespace;
                    let named = native::directory(&directory, &name)?;
                    if native::identity(&named)?.namespace != child_identity {
                        return Err(RuntimeError::conflict(
                            "Directory changed during rooted enumeration",
                        ));
                    }
                    if !seen.insert((child_identity.0, child_identity.1)) {
                        if strict {
                            return Err(RuntimeError::policy(
                                "Repeated directory identity in native runtime storage",
                            ));
                        }
                        omitted += 1;
                        continue;
                    }
                    self.pins.push(Pin {
                        parent: directory.try_clone()?,
                        name: name.clone(),
                        child: child.try_clone()?,
                        identity: child_identity,
                    });
                    self.directories.insert(key.clone(), child.try_clone()?);
                    pending.push((key, child, depth + 1));
                } else if let Ok(file) = native::file(&directory, &name, false) {
                    if walk_ordinary(&file, false)? {
                        let observed = native::identity(&file)?;
                        if observed.links == 1 {
                            let named = native::file(&directory, &name, false)?;
                            if native::identity(&named)? != observed {
                                return Err(RuntimeError::conflict(
                                    "File changed during rooted enumeration",
                                ));
                            }
                            files.push(relative);
                        } else if strict {
                            return Err(RuntimeError::policy("Hard-linked native runtime storage"));
                        }
                    } else if strict {
                        return Err(RuntimeError::policy(
                            "Special native runtime storage refused",
                        ));
                    }
                } else if strict {
                    return Err(RuntimeError::policy(
                        "Linked or unavailable native runtime storage refused",
                    ));
                }
            }
            if native::identity(&directory)?.namespace != identity {
                return Err(RuntimeError::conflict(
                    "Directory changed during rooted enumeration",
                ));
            }
            self.check()?;
        }
        files.sort();
        self.check()?;
        if strict && omitted > 0 {
            return Err(RuntimeError::policy(
                "Native runtime storage exceeds inspection bound",
            ));
        }
        Ok((files, if strict { visited } else { omitted }))
    }
}
#[cfg(unix)]
fn walk_names(directory: &File, limit: usize) -> std::io::Result<(Vec<String>, bool)> {
    use std::ffi::CStr;
    use std::os::fd::IntoRawFd;
    let handle = native::directory(directory, ".")?;
    let raw = handle.into_raw_fd();
    let stream = unsafe { libc::fdopendir(raw) };
    if stream.is_null() {
        let error = std::io::Error::last_os_error();
        unsafe { libc::close(raw) };
        return Err(error);
    }
    struct Stream(*mut libc::DIR);
    impl Drop for Stream {
        fn drop(&mut self) {
            unsafe { libc::closedir(self.0) };
        }
    }
    let stream = Stream(stream);
    let mut names = vec![];
    loop {
        // Clear errno so end-of-directory is distinguishable from a failed read.
        #[cfg(any(target_os = "linux", target_os = "android"))]
        unsafe {
            *libc::__errno_location() = 0;
        }
        #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
        unsafe {
            *libc::__error() = 0;
        }
        let entry = unsafe { libc::readdir(stream.0) };
        if entry.is_null() {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error().unwrap_or(0) != 0 {
                return Err(error);
            }
            break;
        }
        let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        let name = match std::str::from_utf8(bytes) {
            Ok(name) => name,
            Err(_) => continue,
        };
        if name == "." || name == ".." {
            continue;
        }
        if names.len() >= limit {
            return Ok((names, true));
        }
        names.push(name.to_string());
    }
    Ok((names, false))
}
#[cfg(windows)]
fn walk_names(directory: &File, limit: usize) -> std::io::Result<(Vec<String>, bool)> {
    use std::{ffi::c_void, io, os::windows::io::AsRawHandle};
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
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtQueryDirectoryFile(
            file: *mut c_void,
            event: *mut c_void,
            apc: *mut c_void,
            context: *mut c_void,
            status: *mut Status,
            information: *mut c_void,
            length: u32,
            class: i32,
            single: u8,
            name: *const c_void,
            restart: u8,
        ) -> i32;
        fn RtlNtStatusToDosError(status: i32) -> u32;
    }
    let handle = directory.try_clone()?;
    let mut buffer = vec![0u8; 64 * 1024];
    let mut names = vec![];
    let mut restart = 1u8;
    loop {
        let mut status = Status {
            status: 0,
            information: 0,
        };
        let code = unsafe {
            NtQueryDirectoryFile(
                handle.as_raw_handle(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut status,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                12,
                0,
                std::ptr::null_mut(),
                restart,
            )
        };
        restart = 0;
        if code as u32 == 0x80000006 {
            break;
        }
        if code < 0 && code as u32 != 0x80000005 {
            return Err(io::Error::from_raw_os_error(
                unsafe { RtlNtStatusToDosError(code) } as i32,
            ));
        }
        let size = status.information;
        if size < 12 || size > buffer.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid native directory buffer",
            ));
        }
        let mut offset = 0usize;
        loop {
            if offset + 12 > size {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid native directory entry",
                ));
            }
            let next = u32::from_le_bytes(buffer[offset..offset + 4].try_into().unwrap()) as usize;
            let bytes =
                u32::from_le_bytes(buffer[offset + 8..offset + 12].try_into().unwrap()) as usize;
            if !bytes.is_multiple_of(2) || offset + 12 + bytes > size {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid native directory name",
                ));
            }
            let units = buffer[offset + 12..offset + 12 + bytes]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
                .collect::<Vec<_>>();
            if let Ok(name) = String::from_utf16(&units)
                && name != "."
                && name != ".."
            {
                if names.len() >= limit {
                    return Ok((names, true));
                }
                names.push(name);
            }
            if next == 0 {
                break;
            }
            if next < 12 + bytes || offset.checked_add(next).is_none_or(|value| value >= size) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid native directory offset",
                ));
            }
            offset += next;
        }
    }
    Ok((names, false))
}

impl PublicationFiles {
    pub fn remove_empty_directory(
        &mut self,
        relative: &str,
        expected: (u64, u64, u32),
    ) -> Result<()> {
        let (parent, name) = self.parent(relative, false)?;
        let child = native::directory(&parent, &name)?;
        if native::identity(&child)?.namespace != expected {
            return Err(RuntimeError::conflict(
                "Created directory ownership changed",
            ));
        }
        let (names, truncated) = walk_names(&child, 1)?;
        if truncated || !names.is_empty() {
            return Err(RuntimeError::conflict(
                "Created directory is no longer empty",
            ));
        }
        self.check()?;
        walk_remove_directory(&parent, &name, expected)?;
        let key = relative.split('/').map(str::to_string).collect::<Vec<_>>();
        self.directories.remove(&key);
        self.pins.retain(|pin| pin.identity != expected);
        self.check()
    }
}
#[cfg(unix)]
fn walk_remove_directory(parent: &File, name: &str, expected: (u64, u64, u32)) -> Result<()> {
    use std::{ffi::CString, os::fd::AsRawFd};
    let child = native::directory(parent, name)?;
    if native::identity(&child)?.namespace != expected {
        return Err(RuntimeError::conflict(
            "Created directory changed before removal",
        ));
    }
    let name = CString::new(name).map_err(|_| RuntimeError::policy("NUL in created directory"))?;
    if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    parent.sync_all()?;
    Ok(())
}
#[cfg(windows)]
fn walk_remove_directory(parent: &File, name: &str, expected: (u64, u64, u32)) -> Result<()> {
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
        security: Handle,
        quality: Handle,
    }
    #[repr(C)]
    struct Status {
        status: usize,
        information: usize,
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
            ea: Handle,
            ea_len: u32,
        ) -> i32;
        fn NtSetInformationFile(
            file: Handle,
            status: *mut Status,
            data: Handle,
            length: u32,
            class: u32,
        ) -> i32;
        fn RtlNtStatusToDosError(code: i32) -> u32;
    }
    let mut wide = std::ffi::OsStr::new(name).encode_wide().collect::<Vec<_>>();
    if wide.len() > 32760 || wide.contains(&0) {
        return Err(RuntimeError::policy("Invalid created directory name"));
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
    let code = unsafe {
        NtCreateFile(
            &mut handle,
            0x00110081,
            &mut attributes,
            &mut status,
            std::ptr::null_mut(),
            0,
            7,
            1,
            0x00200021,
            std::ptr::null_mut(),
            0,
        )
    };
    if code < 0 {
        return Err(
            io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(code) } as i32).into(),
        );
    }
    let file = unsafe { File::from_raw_handle(handle) };
    if native::identity(&file)?.namespace != expected {
        return Err(RuntimeError::conflict(
            "Created directory changed before removal",
        ));
    }
    let mut disposition = 1u8;
    let code = unsafe {
        NtSetInformationFile(
            handle,
            &mut status,
            (&mut disposition as *mut u8).cast(),
            1,
            13,
        )
    };
    if code < 0 {
        return Err(
            io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(code) } as i32).into(),
        );
    }
    drop(file);
    Ok(())
}
