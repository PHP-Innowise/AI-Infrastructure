//! Native identities for directory inventories and source-frame snapshots.
use crate::Result;
#[cfg(not(unix))]
use crate::RuntimeError;
use std::{
    fs::{self, File},
    path::Path,
};
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeIdentity {
    pub device: u64,
    pub file: u128,
    pub kind: u32,
    pub links: u64,
    pub size: u64,
    pub modified: i128,
    pub changed: i128,
    pub reparse_tag: u32,
    pub attributes: u32,
}
impl NativeIdentity {
    pub fn namespace(&self) -> (u64, u128, u32) {
        (self.device, self.file, self.kind)
    }
    pub fn without_contents(mut self) -> Self {
        self.links = 0;
        self.size = 0;
        self.modified = 0;
        self.changed = 0;
        self.attributes = 0;
        self.reparse_tag = 0;
        self
    }
}
#[cfg(unix)]
pub(super) fn from_metadata(m: &fs::Metadata) -> NativeIdentity {
    use std::os::unix::fs::MetadataExt;
    NativeIdentity {
        device: m.dev(),
        file: m.ino() as u128,
        kind: m.mode(),
        links: m.nlink(),
        size: m.size(),
        modified: m.mtime() as i128 * 1_000_000_000 + m.mtime_nsec() as i128,
        changed: m.ctime() as i128 * 1_000_000_000 + m.ctime_nsec() as i128,
        reparse_tag: 0,
        attributes: 0,
    }
}
pub fn identity(path: &Path) -> Result<Option<NativeIdentity>> {
    #[cfg(unix)]
    {
        let m = match fs::symlink_metadata(path) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        Ok(Some(from_metadata(&m)))
    }
    #[cfg(windows)]
    {
        match nt::open(path) {
            Ok(file) => Ok(Some(nt::identity(&file)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(RuntimeError::unavailable(
            "Native file identity is unavailable on this platform",
        ))
    }
}
pub fn handle_identity(file: &File) -> Result<NativeIdentity> {
    #[cfg(unix)]
    {
        let m = file.metadata()?;
        Ok(from_metadata(&m))
    }
    #[cfg(windows)]
    {
        nt::identity(file)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Err(RuntimeError::unavailable(
            "Native file identity is unavailable on this platform",
        ))
    }
}
pub fn redirecting(path: &Path, metadata: &fs::Metadata) -> Result<bool> {
    if metadata.file_type().is_symlink() {
        return Ok(true);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 == 0 {
            return Ok(false);
        }
        let image = identity(path)?
            .ok_or_else(|| RuntimeError::unavailable("Reparse image disappeared"))?;
        Ok(image.reparse_tag == 0 || [0xa000000c, 0xa0000003].contains(&image.reparse_tag))
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(false)
    }
}
#[cfg(windows)]
mod nt {
    use super::*;
    use std::{
        ffi::c_void,
        os::windows::{
            ffi::OsStrExt,
            io::{AsRawHandle, FromRawHandle},
        },
    };
    type Handle = *mut c_void;
    #[repr(C)]
    struct FileIdInfo {
        volume: u64,
        id: [u8; 16],
    }
    #[repr(C)]
    struct TagInfo {
        attributes: u32,
        tag: u32,
    }
    #[repr(C)]
    struct BasicInfo {
        created: i64,
        accessed: i64,
        written: i64,
        changed: i64,
        attributes: u32,
    }
    #[repr(C)]
    struct StandardInfo {
        allocation: i64,
        size: i64,
        links: u32,
        deleted: u8,
        directory: u8,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *const c_void,
            creation: u32,
            flags: u32,
            template: Handle,
        ) -> Handle;
        fn GetFileInformationByHandleEx(
            handle: Handle,
            class: i32,
            info: *mut c_void,
            size: u32,
        ) -> i32;
    }
    pub fn open(path: &Path) -> std::io::Result<File> {
        let mut name = path.as_os_str().encode_wide().collect::<Vec<_>>();
        if name.contains(&0) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "NUL in native path",
            ));
        }
        name.push(0);
        let raw = unsafe {
            CreateFileW(
                name.as_ptr(),
                0x80,
                7,
                std::ptr::null(),
                3,
                0x00200000 | 0x02000000,
                std::ptr::null_mut(),
            )
        };
        if raw as isize == -1 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { File::from_raw_handle(raw) })
    }
    fn info<T>(file: &File, class: i32) -> Result<T> {
        let mut out = std::mem::MaybeUninit::<T>::zeroed();
        if unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                class,
                out.as_mut_ptr().cast(),
                std::mem::size_of::<T>() as u32,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(unsafe { out.assume_init() })
    }
    pub fn identity(file: &File) -> Result<NativeIdentity> {
        let id: FileIdInfo = info(file, 18)?;
        let tag: TagInfo = info(file, 9)?;
        let basic: BasicInfo = info(file, 0)?;
        let standard: StandardInfo = info(file, 1)?;
        let kind = if tag.attributes & 0x10 != 0 {
            0o040000
        } else {
            0o100000
        };
        Ok(NativeIdentity {
            device: id.volume,
            file: u128::from_le_bytes(id.id),
            kind,
            links: standard.links as u64,
            size: standard.size.max(0) as u64,
            modified: basic.written as i128 * 100,
            changed: basic.changed as i128 * 100,
            reparse_tag: tag.tag,
            attributes: tag.attributes,
        })
    }
}
