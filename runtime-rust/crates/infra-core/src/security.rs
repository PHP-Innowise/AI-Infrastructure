//! Owner-only state storage, using retained handles for permission changes.
use crate::{Result, RuntimeError};
use std::path::Path;
fn denied() -> RuntimeError {
    RuntimeError::policy("Harness state directory is unavailable or insecure.")
}
#[cfg(unix)]
fn secure(path: &Path, directory: bool) -> Result<()> {
    use std::{
        fs::OpenOptions,
        os::{
            fd::AsRawFd,
            unix::fs::{MetadataExt, OpenOptionsExt},
        },
    };
    if directory {
        crate::fs::ensure_directory(path, true)?;
    }
    let mut options = OpenOptions::new();
    options.read(true).custom_flags(
        libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | libc::O_CLOEXEC
            | if directory { libc::O_DIRECTORY } else { 0 },
    );
    let file = options.open(path).map_err(|_| denied())?;
    let metadata = file.metadata().map_err(|_| denied())?;
    if (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
        || metadata.uid() != unsafe { libc::geteuid() }
        || (!directory && metadata.nlink() != 1)
    {
        return Err(denied());
    }
    if unsafe { libc::fchmod(file.as_raw_fd(), if directory { 0o700 } else { 0o600 }) } != 0 {
        return Err(denied());
    }
    Ok(())
}
#[cfg(windows)]
mod windows {
    use super::*;
    use std::{
        ffi::c_void,
        fs::{File, OpenOptions},
        os::windows::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawHandle,
        },
        path::{Component, PathBuf},
        ptr,
    };
    type Handle = *mut c_void;
    type Sid = *mut c_void;
    type Bool = i32;
    const READ_CONTROL: u32 = 0x20000;
    const WRITE_DAC: u32 = 0x40000;
    const WRITE_OWNER: u32 = 0x80000;
    const FILE_READ_ATTRIBUTES: u32 = 0x80;
    const OWNER: u32 = 1;
    const DACL: u32 = 4;
    const PROTECTED: u32 = 0x80000000;
    const BACKUP: u32 = 0x02000000;
    const NO_REPARSE: u32 = 0x00200000;
    #[repr(C)]
    struct Attributes {
        length: u32,
        descriptor: *mut c_void,
        inherit: i32,
    }
    #[repr(C)]
    struct FileInformation {
        attributes: u32,
        creation: [u32; 2],
        access: [u32; 2],
        write: [u32; 2],
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    // Windows SDK BY_HANDLE_FILE_INFORMATION: DWORD fields and three FILETIMEs.
    const _: () = {
        assert!(std::mem::size_of::<FileInformation>() == 52);
        assert!(std::mem::align_of::<FileInformation>() == 4);
        assert!(std::mem::offset_of!(FileInformation, links) == 40);
    };
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SidAttrs {
        sid: Sid,
        attrs: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> Handle;
        fn CloseHandle(handle: Handle) -> Bool;
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
        fn CreateDirectoryW(path: *const u16, attrs: *const Attributes) -> Bool;
        fn GetLastError() -> u32;
        fn GetFileInformationByHandle(file: Handle, info: *mut c_void) -> Bool;
    }
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> Bool;
        fn GetTokenInformation(
            token: Handle,
            kind: u32,
            info: *mut c_void,
            size: u32,
            needed: *mut u32,
        ) -> Bool;
        fn EqualSid(a: Sid, b: Sid) -> Bool;
        fn ConvertSidToStringSidW(sid: Sid, text: *mut *mut u16) -> Bool;
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text: *const u16,
            revision: u32,
            descriptor: *mut *mut c_void,
            size: *mut u32,
        ) -> Bool;
        fn GetSecurityDescriptorDacl(
            descriptor: *mut c_void,
            present: *mut Bool,
            dacl: *mut *mut c_void,
            defaulted: *mut Bool,
        ) -> Bool;
        fn GetSecurityInfo(
            handle: Handle,
            kind: u32,
            information: u32,
            owner: *mut Sid,
            group: *mut Sid,
            dacl: *mut *mut c_void,
            sacl: *mut *mut c_void,
            descriptor: *mut *mut c_void,
        ) -> u32;
        fn SetSecurityInfo(
            handle: Handle,
            kind: u32,
            information: u32,
            owner: Sid,
            group: Sid,
            dacl: *mut c_void,
            sacl: *mut c_void,
        ) -> u32;
    }
    struct Memory(*mut c_void);
    impl Drop for Memory {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    LocalFree(self.0);
                }
            }
        }
    }
    struct Token(Handle);
    impl Drop for Token {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }
    struct User {
        _user: Vec<u8>,
        _owner: Vec<u8>,
        sid: Sid,
        owner: Sid,
        descriptor: Memory,
    }
    fn token_info(token: Handle, kind: u32) -> Result<Vec<u8>> {
        let mut needed = 0;
        unsafe {
            GetTokenInformation(token, kind, ptr::null_mut(), 0, &mut needed);
        }
        if needed == 0 || needed > 65536 {
            return Err(denied());
        }
        let mut bytes = vec![0u8; needed as usize];
        if unsafe {
            GetTokenInformation(token, kind, bytes.as_mut_ptr().cast(), needed, &mut needed)
        } == 0
        {
            return Err(denied());
        }
        Ok(bytes)
    }
    fn user() -> Result<User> {
        let mut handle = ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), 8, &mut handle) } == 0 {
            return Err(denied());
        }
        let token = Token(handle);
        let user = token_info(token.0, 1)?;
        let owner = token_info(token.0, 4)?;
        if user.len() < std::mem::size_of::<SidAttrs>() || owner.len() < std::mem::size_of::<Sid>()
        {
            return Err(denied());
        }
        let sid = unsafe { ptr::read_unaligned(user.as_ptr().cast::<SidAttrs>()) }.sid;
        let owner_sid = unsafe { ptr::read_unaligned(owner.as_ptr().cast::<Sid>()) };
        if sid.is_null() || owner_sid.is_null() {
            return Err(denied());
        }
        let mut text = ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
            return Err(denied());
        }
        let memory = Memory(text.cast());
        let mut length = 0;
        while length < 4096 && unsafe { *text.add(length) } != 0 {
            length += 1;
        }
        if length == 4096 {
            return Err(denied());
        }
        let text = String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) })
            .map_err(|_| denied())?;
        drop(memory);
        let sddl = format!("O:{text}D:P(A;OICI;FA;;;{text})(A;OICI;FA;;;SY)")
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let mut descriptor = ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(denied());
        }
        Ok(User {
            _user: user,
            _owner: owner,
            sid,
            owner: owner_sid,
            descriptor: Memory(descriptor),
        })
    }
    fn reparse(file: &File) -> Result<bool> {
        let meta = file.metadata().map_err(|_| denied())?;
        Ok(meta.file_attributes() & 0x400 != 0)
    }
    fn open(path: &Path, leaf: bool, directory: bool) -> Result<File> {
        let access =
            READ_CONTROL | FILE_READ_ATTRIBUTES | if leaf { WRITE_DAC | WRITE_OWNER } else { 0 };
        let mut options = OpenOptions::new();
        options
            .access_mode(access)
            .share_mode(1 | 2)
            .custom_flags(NO_REPARSE | if directory { BACKUP } else { 0 });
        let file = options.open(path).map_err(|_| denied())?;
        let meta = file.metadata().map_err(|_| denied())?;
        if reparse(&file)? || (directory && !meta.is_dir()) || (!directory && !meta.is_file()) {
            return Err(denied());
        }
        Ok(file)
    }
    fn protect(file: &File, user: &User) -> Result<()> {
        let mut owner = ptr::null_mut();
        let mut descriptor = ptr::null_mut();
        if unsafe {
            GetSecurityInfo(
                file.as_raw_handle().cast(),
                1,
                OWNER,
                &mut owner,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut descriptor,
            )
        } != 0
        {
            return Err(denied());
        }
        let owner_memory = Memory(descriptor);
        if owner.is_null()
            || (unsafe { EqualSid(owner, user.sid) } == 0
                && unsafe { EqualSid(owner, user.owner) } == 0)
        {
            return Err(denied());
        }
        drop(owner_memory);
        let mut present = 0;
        let mut defaulted = 0;
        let mut dacl = ptr::null_mut();
        if unsafe {
            GetSecurityDescriptorDacl(user.descriptor.0, &mut present, &mut dacl, &mut defaulted)
        } == 0
            || present == 0
            || dacl.is_null()
        {
            return Err(denied());
        }
        if unsafe {
            SetSecurityInfo(
                file.as_raw_handle().cast(),
                1,
                OWNER | DACL | PROTECTED,
                user.sid,
                ptr::null_mut(),
                dacl,
                ptr::null_mut(),
            )
        } != 0
        {
            return Err(denied());
        }
        Ok(())
    }
    pub(super) fn secure_handle(file: &File, directory: bool) -> Result<()> {
        let metadata = file.metadata().map_err(|_| denied())?;
        if reparse(file)?
            || (directory && !metadata.is_dir())
            || (!directory && !metadata.is_file())
        {
            return Err(denied());
        }
        if !directory {
            let mut info: FileInformation = unsafe { std::mem::zeroed() };
            if unsafe {
                GetFileInformationByHandle(
                    file.as_raw_handle().cast(),
                    (&mut info as *mut FileInformation).cast(),
                )
            } == 0
                || info.links != 1
            {
                return Err(denied());
            }
        }
        protect(file, &user()?)
    }
    pub(super) fn with_creation_descriptor<T, F>(create: F) -> std::io::Result<T>
    where
        F: FnOnce(*mut c_void) -> std::io::Result<T>,
    {
        let owner = user().map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Private creation credentials unavailable",
            )
        })?;
        create(owner.descriptor.0)
    }
    pub(super) fn secure(path: &Path, directory: bool) -> Result<()> {
        let path = std::path::absolute(path).map_err(|_| denied())?;
        let user = user()?;
        let mut current = PathBuf::new();
        let mut handles = Vec::new();
        let components = path.components().collect::<Vec<_>>();
        if components
            .iter()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err(denied());
        }
        let mut normal = components
            .iter()
            .filter(|c| matches!(c, Component::Normal(_)))
            .count();
        for component in components {
            current.push(component.as_os_str());
            match component {
                Component::Prefix(_) => continue,
                Component::RootDir => {
                    handles.push(open(&current, false, true)?);
                    continue;
                }
                Component::Normal(_) => {
                    normal -= 1;
                    let leaf = normal == 0;
                    let is_directory = !leaf || directory;
                    if !current.exists() && is_directory {
                        let text = current
                            .as_os_str()
                            .encode_wide()
                            .chain(Some(0))
                            .collect::<Vec<_>>();
                        let attributes = Attributes {
                            length: std::mem::size_of::<Attributes>() as u32,
                            descriptor: user.descriptor.0,
                            inherit: 0,
                        };
                        if unsafe { CreateDirectoryW(text.as_ptr(), &attributes) } == 0
                            && unsafe { GetLastError() } != 183
                        {
                            return Err(denied());
                        }
                    }
                    let file = open(&current, leaf, is_directory)?;
                    if leaf {
                        if !directory {
                            let mut info: FileInformation = unsafe { std::mem::zeroed() };
                            if unsafe {
                                GetFileInformationByHandle(
                                    file.as_raw_handle().cast(),
                                    (&mut info as *mut FileInformation).cast(),
                                )
                            } == 0
                                || info.links != 1
                            {
                                return Err(denied());
                            }
                        }
                        protect(&file, &user)?;
                    }
                    handles.push(file);
                }
                _ => return Err(denied()),
            }
        }
        if handles.is_empty() {
            return Err(denied());
        }
        Ok(())
    }
}
#[cfg(windows)]
fn secure(path: &Path, directory: bool) -> Result<()> {
    windows::secure(path, directory)
}
#[cfg(not(any(unix, windows)))]
fn secure(_path: &Path, _directory: bool) -> Result<()> {
    Err(denied())
}

#[cfg(unix)]
#[allow(
    clippy::unnecessary_cast,
    reason = "libc mode_t has platform-specific width"
)]
pub(crate) fn secure_private_handle(file: &std::fs::File, directory: bool) -> Result<()> {
    use std::os::{fd::AsRawFd, unix::fs::MetadataExt};
    let metadata = file.metadata().map_err(|_| denied())?;
    if (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
        || metadata.uid() != unsafe { libc::geteuid() }
        || (!directory && metadata.nlink() != 1)
    {
        return Err(denied());
    }
    let mode = if directory {
        0o700
    } else {
        metadata.mode() & 0o700
    };
    // A chmod to the mode the object already has still moves its change time,
    // and custody checks compare that time: two launches protecting one
    // already-private state at once refused each other's files as changed.
    if metadata.mode() & 0o7777 == mode {
        return Ok(());
    }
    if unsafe { libc::fchmod(file.as_raw_fd(), mode as libc::mode_t) } != 0 {
        return Err(denied());
    }
    Ok(())
}
#[cfg(windows)]
pub(crate) fn secure_private_handle(file: &std::fs::File, directory: bool) -> Result<()> {
    windows::secure_handle(file, directory)
}
#[cfg(not(any(unix, windows)))]
pub(crate) fn secure_private_handle(_: &std::fs::File, _: bool) -> Result<()> {
    Err(denied())
}

/// Keep the protected owner-and-SYSTEM descriptor alive for one native creation.
/// The pointer is borrowed only for the callback and must not be retained.
#[cfg(windows)]
pub fn with_private_creation_descriptor<T, F>(create: F) -> std::io::Result<T>
where
    F: FnOnce(*mut std::ffi::c_void) -> std::io::Result<T>,
{
    windows::with_creation_descriptor(create)
}

pub fn secure_private_dir(path: &Path) -> Result<()> {
    secure(path, true)
}
pub fn secure_private_file(path: &Path) -> Result<()> {
    secure(path, false)
}
