//! No-follow final-link observations for baseline-only publication paths.
use super::PublicationFiles;
use crate::{Result, RuntimeError, query::sha256};
use serde_json::{Value, json as object};
use std::fs::File;
impl PublicationFiles {
    pub fn watch_image(&mut self, relative: &str, limit: usize) -> Result<Value> {
        let (parent, name) = match self.parent(relative, false) {
            Ok(value) => value,
            Err(error) if error.kind == crate::ErrorKind::NotFound => {
                self.check()?;
                return Ok(object!({"state":"missing","identity":null}));
            }
            Err(error) => return Err(error),
        };
        self.check()?;
        if let Some((target, identity)) = observe_link(&parent, &name)? {
            self.check()?;
            return Ok(
                object!({"state":"symlink","target":target,"sha256":sha256(target.as_bytes()),"identity":identity}),
            );
        }
        let before = self.image_identity(relative)?;
        let raw = self.read(relative, limit)?;
        let mode = self.read_mode(relative)?;
        let after = self.image_identity(relative)?;
        if before != after {
            return Err(RuntimeError::conflict(
                "Watched regular image changed during read",
            ));
        }
        let value = if let Some(raw) = raw {
            object!({"state":"file","sha256":sha256(&raw),"mode":mode,"identity":after})
        } else {
            object!({"state":"missing","identity":null})
        };
        self.check()?;
        Ok(value)
    }
}
#[cfg(unix)]
fn observe_link(parent: &File, name: &str) -> Result<Option<(String, Value)>> {
    use std::{ffi::CString, io, os::fd::AsRawFd};
    let name = CString::new(name).map_err(|_| RuntimeError::policy("NUL in watched name"))?;
    let stat = || -> io::Result<libc::stat> {
        let mut value = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                name.as_ptr(),
                value.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } < 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { value.assume_init() })
    };
    let before = match stat() {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if before.st_mode & libc::S_IFMT != libc::S_IFLNK {
        return Ok(None);
    }
    let identity = |value: &libc::stat| {
        (
            i128::from(value.st_dev) as u64,
            i128::from(value.st_ino) as u64,
            i128::from(value.st_mode) as u32,
            i128::from(value.st_nlink) as u64,
            i128::from(value.st_size) as i64,
        )
    };
    let mut buffer = vec![0u8; 64 * 1024];
    let size = unsafe {
        libc::readlinkat(
            parent.as_raw_fd(),
            name.as_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    if size < 0 {
        return Err(io::Error::last_os_error().into());
    }
    if size as usize >= buffer.len() {
        return Err(RuntimeError::policy("Watched symlink exceeds bound"));
    }
    buffer.truncate(size as usize);
    if identity(&before) != identity(&stat()?) {
        return Err(RuntimeError::conflict("Watched link identity changed"));
    }
    let target = String::from_utf8(buffer)
        .map_err(|_| RuntimeError::policy("Watched link target is not UTF-8"))?;
    Ok(Some((target, object!(identity(&before)))))
}
#[cfg(windows)]
fn observe_link(parent: &File, name: &str) -> Result<Option<(String, Value)>> {
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
    #[repr(C)]
    struct Time {
        low: u32,
        high: u32,
    }
    #[repr(C)]
    struct Info {
        attributes: u32,
        created: Time,
        accessed: Time,
        written: Time,
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    // Windows SDK BY_HANDLE_FILE_INFORMATION: DWORD fields and three FILETIMEs.
    const _: () = {
        assert!(std::mem::size_of::<Info>() == 52);
        assert!(std::mem::align_of::<Info>() == 4);
        assert!(std::mem::offset_of!(Info, links) == 40);
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
            ea: Handle,
            ea_len: u32,
        ) -> i32;
        fn RtlNtStatusToDosError(code: i32) -> u32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(file: Handle, out: *mut c_void) -> i32;
        fn DeviceIoControl(
            file: Handle,
            code: u32,
            input: Handle,
            input_len: u32,
            output: Handle,
            output_len: u32,
            returned: *mut u32,
            overlapped: Handle,
        ) -> i32;
    }
    let mut wide = std::ffi::OsStr::new(name).encode_wide().collect::<Vec<_>>();
    if wide.len() > 32760 || wide.contains(&0) {
        return Err(RuntimeError::policy("Invalid watched name"));
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
            0x00100080,
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
    };
    if code < 0 {
        let error = io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(code) } as i32);
        if error.kind() == io::ErrorKind::NotFound {
            return Ok(None);
        }
        return Err(error.into());
    }
    let file = unsafe { File::from_raw_handle(handle) };
    let information = |file: &File| -> io::Result<Info> {
        let mut value = std::mem::MaybeUninit::<Info>::uninit();
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), value.as_mut_ptr().cast()) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { value.assume_init() })
    };
    let before = information(&file)?;
    if before.attributes & 0x400 == 0 {
        return Ok(None);
    }
    let mut buffer = vec![0u8; 16 * 1024];
    let mut returned = 0u32;
    if unsafe {
        DeviceIoControl(
            handle,
            0x000900A8,
            std::ptr::null_mut(),
            0,
            buffer.as_mut_ptr().cast(),
            buffer.len() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error().into());
    }
    buffer.truncate(returned as usize);
    if buffer.len() < 16 {
        return Err(RuntimeError::policy("Invalid watched reparse data"));
    }
    let tag = u32::from_le_bytes(buffer[..4].try_into().unwrap());
    let base = match tag {
        0xA000000C => 20usize,
        0xA0000003 => 16usize,
        _ => return Err(RuntimeError::policy("Unsupported watched reparse point")),
    };
    let offset = u16::from_le_bytes(buffer[8..10].try_into().unwrap()) as usize;
    let length = u16::from_le_bytes(buffer[10..12].try_into().unwrap()) as usize;
    if !length.is_multiple_of(2) || base + offset + length > buffer.len() {
        return Err(RuntimeError::policy("Invalid watched reparse target"));
    }
    let wide = buffer[base + offset..base + offset + length]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    let target = String::from_utf16(&wide)
        .map_err(|_| RuntimeError::policy("Watched reparse target is not Unicode"))?;
    let identity = |value: &Info| {
        (
            value.volume as u64,
            ((value.index_high as u64) << 32) | value.index_low as u64,
            value.attributes,
            value.links as u64,
            ((value.size_high as u64) << 32) | value.size_low as u64,
        )
    };
    let after = information(&file)?;
    let mut named_handle = std::ptr::null_mut();
    let named_code = unsafe {
        NtCreateFile(
            &mut named_handle,
            0x00100080,
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
    };
    if named_code < 0 {
        return Err(RuntimeError::conflict("Watched reparse name disappeared"));
    }
    let named = unsafe { File::from_raw_handle(named_handle) };
    if identity(&before) != identity(&information(&named)?) || identity(&before) != identity(&after)
    {
        return Err(RuntimeError::conflict("Watched reparse identity changed"));
    }
    Ok(Some((target, object!(identity(&before)))))
}
