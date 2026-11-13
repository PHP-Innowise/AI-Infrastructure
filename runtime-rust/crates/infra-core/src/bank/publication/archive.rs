//! Private merged-chat archives. Every directory and effect stays beneath pinned descriptors.
use super::{PublicationFiles, native};
use crate::{Result, RuntimeError};
use std::fs::File;
impl PublicationFiles {
    fn archive_folder(&mut self, relative: &str, create: bool) -> Result<File> {
        if relative.is_empty() {
            self.check()?;
            return self
                .directories
                .get(&Vec::<String>::new())
                .ok_or_else(|| RuntimeError::unavailable("Archive root unavailable"))?
                .try_clone()
                .map_err(Into::into);
        }
        let (parent, _) = self.parent(&format!("{relative}/.archive-custody"), create)?;
        self.check()?;
        Ok(parent)
    }
    pub fn archive_create_directory(&mut self, relative: &str, exclusive: bool) -> Result<()> {
        let (parent, name) = self.parent(relative, true)?;
        self.check()?;
        match native::mkdir(&parent, &name) {
            Ok(()) => {}
            Err(e) if !exclusive && e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        let child = self.archive_folder(relative, false)?;
        #[cfg(not(unix))]
        let _custody = child;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if child.metadata()?.permissions().mode() & 0o077 != 0 {
                return Err(RuntimeError::policy("Archive directory must be private"));
            }
        }
        self.check()
    }
    pub fn archive_entries(&mut self, relative: &str, limit: usize) -> Result<Vec<String>> {
        let parent = self.archive_folder(relative, false)?;
        let names = super::cursor::cursor_names(&parent, Some(limit))?;
        self.check()?;
        Ok(names)
    }
    pub fn archive_directory_exists(&mut self, relative: &str) -> Result<bool> {
        match self.archive_folder(relative, false) {
            Ok(_) => Ok(true),
            Err(e) if e.kind == crate::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }
    pub fn archive_remove_entry(&mut self, relative: &str) -> Result<bool> {
        let (parent, name) = match self.parent(relative, false) {
            Ok(p) => p,
            Err(e) if e.kind == crate::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        };
        self.check()?;
        match archive_native::remove(&parent, &name, false) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
        }
        #[cfg(unix)]
        parent.sync_all()?;
        self.check()?;
        Ok(true)
    }
    pub fn archive_remove_empty_directory(&mut self, relative: &str) -> Result<bool> {
        let (parent, name) = match self.parent(relative, false) {
            Ok(p) => p,
            Err(e) if e.kind == crate::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        };
        self.check()?;
        let child = match native::directory(&parent, &name) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        let identity = native::identity(&child)?.namespace;
        if !super::cursor::cursor_names(&child, Some(1))?.is_empty() {
            return Ok(false);
        }
        if native::identity(&native::directory(&parent, &name)?)?.namespace != identity {
            return Err(RuntimeError::conflict(
                "Archive directory changed before removal",
            ));
        }
        match archive_native::remove(&parent, &name, true) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => return Ok(false),
            Err(e) => return Err(e.into()),
        }
        let key = relative.split('/').map(str::to_owned).collect::<Vec<_>>();
        let removed = self
            .directories
            .iter()
            .filter(|(k, _)| k.starts_with(&key))
            .filter_map(|(_, f)| native::identity(f).ok().map(|v| v.namespace))
            .collect::<Vec<_>>();
        self.directories.retain(|k, _| !k.starts_with(&key));
        self.pins.retain(|pin| !removed.contains(&pin.identity));
        #[cfg(unix)]
        parent.sync_all()?;
        self.check()?;
        Ok(true)
    }
}
#[cfg(unix)]
mod archive_native {
    use std::{ffi::CString, fs::File, io, os::fd::AsRawFd};
    pub fn remove(parent: &File, name: &str, directory: bool) -> io::Result<()> {
        let name = CString::new(name)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid archive entry"))?;
        let rc = unsafe {
            libc::unlinkat(
                parent.as_raw_fd(),
                name.as_ptr(),
                if directory { libc::AT_REMOVEDIR } else { 0 },
            )
        };
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}
#[cfg(windows)]
mod archive_native {
    use std::{
        ffi::c_void,
        fs::File,
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
    fn admitted(status: i32) -> io::Result<()> {
        if status < 0 {
            Err(io::Error::from_raw_os_error(
                unsafe { RtlNtStatusToDosError(status) } as i32,
            ))
        } else {
            Ok(())
        }
    }
    pub fn remove(parent: &File, name: &str, directory: bool) -> io::Result<()> {
        let mut wide = std::ffi::OsStr::new(name).encode_wide().collect::<Vec<_>>();
        if wide.is_empty() || wide.len() > 32760 || wide.contains(&0) || name.contains(['\\', '/'])
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid archive entry",
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
                0x00110080,
                &mut attributes,
                &mut status,
                std::ptr::null_mut(),
                0,
                7,
                1,
                0x00200020,
                std::ptr::null_mut(),
                0,
            )
        })?;
        let file = unsafe { File::from_raw_handle(handle) };
        let mut basic = std::mem::MaybeUninit::<Basic>::uninit();
        let mut standard = std::mem::MaybeUninit::<Standard>::uninit();
        admitted(unsafe {
            NtQueryInformationFile(
                handle,
                &mut status,
                basic.as_mut_ptr().cast(),
                std::mem::size_of::<Basic>() as u32,
                4,
            )
        })?;
        admitted(unsafe {
            NtQueryInformationFile(
                handle,
                &mut status,
                standard.as_mut_ptr().cast(),
                std::mem::size_of::<Standard>() as u32,
                5,
            )
        })?;
        let basic = unsafe { basic.assume_init() };
        let standard = unsafe { standard.assume_init() };
        let reparse = basic.attributes & 0x400 != 0;
        if directory && (standard.directory == 0 || reparse)
            || !directory && standard.directory != 0 && !reparse
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Archive entry type changed",
            ));
        }
        // FILE_OPEN_REPARSE_POINT and handle disposition remove the final namespace entry itself.
        let mut deletion = 1u8;
        admitted(unsafe {
            NtSetInformationFile(
                file.as_raw_handle(),
                &mut status,
                (&mut deletion as *mut u8).cast(),
                1,
                13,
            )
        })
    }
}

/// Append-tolerant reader for provider-owned local journals. The opened and named namespace must stay pinned.
pub struct ArchiveReader {
    custody: PublicationFiles,
    parent: File,
    name: String,
    file: File,
    namespace: (u64, u64, u32),
    links: u64,
}
impl ArchiveReader {
    pub fn size(&self) -> Result<u64> {
        self.check()?;
        Ok(self.file.metadata()?.len())
    }
    pub fn check(&self) -> Result<()> {
        self.custody.check()?;
        let opened = native::identity(&self.file)?;
        let named = native::identity(&native::file(&self.parent, &self.name, false)?)?;
        if opened.namespace != self.namespace
            || named.namespace != self.namespace
            || opened.links != self.links
            || named.links != self.links
            || self.links != 1
        {
            return Err(RuntimeError::conflict("Local journal identity changed"));
        }
        Ok(())
    }
    pub fn identity(&self) -> (u64, u64, u32) {
        self.namespace
    }
    pub fn read(&mut self, offset: u64, limit: usize) -> Result<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        if limit > 4 * 1024 * 1024 {
            return Err(RuntimeError::invalid("Journal range bound exceeded"));
        }
        self.check()?;
        let size = self.file.metadata()?.len();
        if offset > size {
            return Err(RuntimeError::conflict("Local journal was truncated"));
        }
        self.file.seek(SeekFrom::Start(offset))?;
        let mut bytes = vec![];
        Read::by_ref(&mut self.file)
            .take(limit as u64)
            .read_to_end(&mut bytes)?;
        self.check()?;
        if self.file.metadata()?.len() < offset + bytes.len() as u64 {
            return Err(RuntimeError::conflict(
                "Local journal was truncated during read",
            ));
        }
        Ok(bytes)
    }
    pub fn prefix_line(&mut self, limit: usize) -> Result<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        if limit > 4 * 1024 * 1024 {
            return Err(RuntimeError::invalid("Journal line bound exceeded"));
        }
        self.check()?;
        self.file.seek(SeekFrom::Start(0))?;
        let mut out = vec![];
        let mut byte = [0u8; 1];
        while out.len() < limit && self.file.read(&mut byte)? != 0 {
            out.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
        }
        self.check()?;
        Ok(out)
    }
    pub fn tail(&mut self, after_header: u64, limit: usize) -> Result<(u64, Vec<u8>)> {
        use std::io::{Read, Seek, SeekFrom};
        if limit > 4 * 1024 * 1024 {
            return Err(RuntimeError::invalid("Journal tail bound exceeded"));
        }
        self.check()?;
        let length = self.file.metadata()?.len();
        let mut start = after_header.max(length.saturating_sub(limit as u64));
        self.file.seek(SeekFrom::Start(start))?;
        if start > after_header {
            let mut byte = [0u8; 1];
            let mut skipped = 0;
            while skipped < limit && self.file.read(&mut byte)? != 0 {
                start += 1;
                skipped += 1;
                if byte[0] == b'\n' {
                    break;
                }
            }
        }
        let mut bytes = vec![];
        Read::by_ref(&mut self.file)
            .take(limit as u64)
            .read_to_end(&mut bytes)?;
        self.check()?;
        Ok((start, bytes))
    }
}
impl PublicationFiles {
    pub fn archive_reader(mut self, relative: &str) -> Result<Option<ArchiveReader>> {
        let (parent, name) = match self.parent(relative, false) {
            Ok(v) => v,
            Err(e) if e.kind == crate::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let file = match native::file(&parent, &name, false) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let image = native::identity(&file)?;
        if !file.metadata()?.is_file() || image.links != 1 {
            return Err(RuntimeError::policy(
                "Local journal is not an ordinary single-link file",
            ));
        }
        let reader = ArchiveReader {
            custody: self,
            parent,
            name,
            file,
            namespace: image.namespace,
            links: image.links,
        };
        reader.check()?;
        Ok(Some(reader))
    }
}
