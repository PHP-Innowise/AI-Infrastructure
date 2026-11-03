//! Python-compatible spelling for admitted native roots and queue identities.
//! The caller's explicit Windows extended prefix is preserved. A prefix added
//! by canonicalize is removed only when the ordinary drive/UNC name proves
//! equivalent (or has the same missing-path error as the original request).
use crate::Result;
use std::path::{Path, PathBuf};

pub fn python_realpath(path: &Path) -> Result<PathBuf> {
    #[cfg(windows)]
    {
        Ok(windows::realpath(path)?)
    }
    #[cfg(not(windows))]
    {
        super::canonical_or_missing(path)
    }
}
/// Preserve original root-prefix intent for an already resolved native root.
pub fn python_realpath_spelling(original: &Path, resolved: PathBuf) -> Result<PathBuf> {
    #[cfg(windows)]
    {
        Ok(windows::spelling(original, resolved, None))
    }
    #[cfg(not(windows))]
    {
        let _ = original;
        Ok(resolved)
    }
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::{
        ffi::{OsStr, OsString},
        fs, io,
        os::windows::ffi::{OsStrExt, OsStringExt},
        path::{Component, Path, PathBuf},
    };
    type Handle = *mut c_void;
    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[repr(C)]
    struct FindData {
        attributes: u32,
        created: FileTime,
        accessed: FileTime,
        written: FileTime,
        size_high: u32,
        size_low: u32,
        reserved0: u32,
        reserved1: u32,
        name: [u16; 260],
        alternate: [u16; 14],
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFullPathNameW(
            name: *const u16,
            size: u32,
            buffer: *mut u16,
            file_part: *mut *mut u16,
        ) -> u32;
        fn FindFirstFileW(name: *const u16, data: *mut FindData) -> Handle;
        fn FindClose(handle: Handle) -> i32;
        fn CompareStringOrdinal(
            a: *const u16,
            a_len: i32,
            b: *const u16,
            b_len: i32,
            ignore_case: i32,
        ) -> i32;
    }
    fn wide(path: &Path) -> io::Result<Vec<u16>> {
        let mut text = path.as_os_str().encode_wide().collect::<Vec<_>>();
        if text.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "NUL in native root",
            ));
        }
        text.push(0);
        Ok(text)
    }
    fn prefixed(path: &Path) -> bool {
        let mut units = path.as_os_str().encode_wide().take(4).map(|unit| {
            if unit == b'/' as u16 {
                b'\\' as u16
            } else {
                unit
            }
        });
        units.next() == Some(b'\\' as u16)
            && units.next() == Some(b'\\' as u16)
            && units.next() == Some(b'?' as u16)
            && units.next() == Some(b'\\' as u16)
    }
    fn normalize(path: &Path) -> io::Result<PathBuf> {
        let text = OsString::from_wide(
            &path
                .as_os_str()
                .encode_wide()
                .map(|unit| {
                    if unit == b'/' as u16 {
                        b'\\' as u16
                    } else {
                        unit
                    }
                })
                .collect::<Vec<_>>(),
        );
        let input = wide(Path::new(&text))?;
        let mut size = 260usize;
        let full = loop {
            let mut buffer = vec![0u16; size];
            let count = unsafe {
                GetFullPathNameW(
                    input.as_ptr(),
                    buffer.len() as u32,
                    buffer.as_mut_ptr(),
                    std::ptr::null_mut(),
                )
            };
            if count == 0 {
                return Err(io::Error::last_os_error());
            }
            if (count as usize) < buffer.len() {
                buffer.truncate(count as usize);
                break PathBuf::from(OsString::from_wide(&buffer));
            }
            size = (count as usize).checked_add(1).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "Native root is too long")
            })?;
            if size > 32768 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Native root is too long",
                ));
            }
        };
        // Verbatim Rust paths keep literal dot components; ntpath.normpath
        // removes them before asking the operating system for a real name.
        let mut result = PathBuf::new();
        for part in full.components() {
            match part {
                Component::CurDir => {}
                Component::ParentDir => {
                    result.pop();
                }
                Component::Normal(name) if name == OsStr::new(".") => {}
                Component::Normal(name) if name == OsStr::new("..") => {
                    result.pop();
                }
                _ => result.push(part.as_os_str()),
            }
        }
        Ok(result)
    }
    fn append(mut base: PathBuf, tail: &[OsString]) -> PathBuf {
        // PathBuf::push would reinterpret a colon in a missing component as a
        // drive prefix. ntpath's non-strict resolver deliberately joins raw.
        let mut units = base.as_os_str().encode_wide().collect::<Vec<_>>();
        for name in tail.iter().rev() {
            if units.last() != Some(&(b'\\' as u16)) {
                units.push(b'\\' as u16)
            }
            units.extend(name.encode_wide());
        }
        base = PathBuf::from(OsString::from_wide(&units));
        base
    }
    fn same_name(a: &Path, b: &Path) -> bool {
        let a = a.as_os_str().encode_wide().collect::<Vec<_>>();
        let b = b.as_os_str().encode_wide().collect::<Vec<_>>();
        (unsafe { CompareStringOrdinal(a.as_ptr(), a.len() as i32, b.as_ptr(), b.len() as i32, 1) })
            == 2
    }
    fn readlink_deep(path: &Path) -> io::Result<PathBuf> {
        let mut current = path.to_path_buf();
        let mut seen = Vec::<PathBuf>::new();
        loop {
            if seen.iter().any(|old| same_name(old, &current)) {
                return Ok(current);
            }
            seen.push(current.clone());
            match fs::read_link(&current) {
                Ok(target) => {
                    if target.is_absolute() {
                        current = target;
                    } else {
                        if !fs::symlink_metadata(&current)?.file_type().is_symlink() {
                            return Ok(current);
                        }
                        let parent = current.parent().unwrap_or(Path::new(""));
                        current = normalize(&parent.join(target))?;
                    }
                }
                Err(error)
                    if error.raw_os_error().is_some_and(|code| {
                        [1, 2, 3, 5, 21, 32, 50, 67, 87, 4390, 4392, 4393].contains(&code)
                    }) =>
                {
                    return Ok(current);
                }
                Err(error)
                    if error.kind() == io::ErrorKind::InvalidInput
                        && error.raw_os_error().is_none() =>
                {
                    return Ok(current);
                }
                Err(error) => return Err(error),
            }
        }
    }
    fn found_name(path: &Path) -> io::Result<OsString> {
        let input = wide(path)?;
        let mut data = std::mem::MaybeUninit::<FindData>::zeroed();
        let handle = unsafe { FindFirstFileW(input.as_ptr(), data.as_mut_ptr()) };
        if handle as isize == -1 {
            return Err(io::Error::last_os_error());
        }
        unsafe { FindClose(handle) };
        let data = unsafe { data.assume_init() };
        let end = data
            .name
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(data.name.len());
        Ok(OsString::from_wide(&data.name[..end]))
    }
    fn nonstrict(path: &Path) -> io::Result<PathBuf> {
        let mut current = path.to_path_buf();
        let mut tail = Vec::<OsString>::new();
        loop {
            match fs::canonicalize(&current) {
                Ok(target) => return Ok(append(target, &tail)),
                Err(error) => {
                    let Some(code) = error.raw_os_error() else {
                        return Err(error);
                    };
                    if ![
                        1, 2, 3, 5, 21, 32, 50, 53, 65, 67, 87, 123, 161, 1005, 1920, 1921,
                    ]
                    .contains(&code)
                    {
                        return Err(error);
                    }
                    if let Ok(target) = readlink_deep(&current)
                        && target != current
                    {
                        return Ok(append(target, &tail));
                    }
                    let Some(name) = current.file_name().map(OsStr::to_os_string) else {
                        return Ok(append(current, &tail));
                    };
                    let name = if [1, 5, 32, 50, 87, 1920, 1921].contains(&code) {
                        found_name(&current).unwrap_or(name)
                    } else {
                        name
                    };
                    tail.push(name);
                    if !current.pop() {
                        return Ok(append(current, &tail));
                    }
                }
            }
        }
    }
    pub fn spelling(original: &Path, resolved: PathBuf, initial_error: Option<i32>) -> PathBuf {
        if prefixed(original) {
            return resolved;
        }
        let units = resolved.as_os_str().encode_wide().collect::<Vec<_>>();
        if !units.starts_with(&[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16]) {
            return resolved;
        }
        let candidate = if units.get(4..8)
            == Some(&[b'U' as u16, b'N' as u16, b'C' as u16, b'\\' as u16] as &[u16])
        {
            let mut ordinary = vec![b'\\' as u16, b'\\' as u16];
            ordinary.extend_from_slice(&units[8..]);
            Some(ordinary)
        } else if units.get(5) == Some(&(b':' as u16)) && units.get(6) == Some(&(b'\\' as u16)) {
            Some(units[4..].to_vec())
        } else {
            None
        };
        let Some(candidate) = candidate else {
            return resolved;
        };
        let candidate = PathBuf::from(OsString::from_wide(&candidate));
        match fs::canonicalize(&candidate) {
            Ok(actual) if actual.as_os_str() == resolved.as_os_str() => candidate,
            Err(error) if initial_error.is_some() && error.raw_os_error() == initial_error => {
                candidate
            }
            _ => resolved,
        }
    }
    pub fn realpath(path: &Path) -> io::Result<PathBuf> {
        let original = path;
        let normalized = normalize(path)?;
        let (resolved, initial_error) = match fs::canonicalize(&normalized) {
            Ok(path) => (path, None),
            Err(error) => {
                let code = error.raw_os_error();
                (nonstrict(&normalized)?, code)
            }
        };
        Ok(spelling(original, resolved, initial_error))
    }
}
