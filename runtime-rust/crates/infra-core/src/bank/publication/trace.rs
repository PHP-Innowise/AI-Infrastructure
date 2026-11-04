//! Optional POSIX trace sidecars. Finalization edits an existing inode, so a
//! retention prune cannot be undone by a later renderer. Windows has no
//! canonical descriptor-relative trace profile and returns unavailable.
use super::{PublicationFiles, cursor::cursor_names, native};
use crate::{ErrorKind, Result, RuntimeError};
#[cfg(unix)]
use std::io::{Seek, SeekFrom};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};
pub struct TraceFiles {
    files: PublicationFiles,
}
impl TraceFiles {
    pub fn new(root: &Path) -> Result<Self> {
        if !cfg!(unix) {
            return Err(RuntimeError::unavailable(
                "Descriptor-relative trace custody unavailable",
            ));
        }
        Ok(Self {
            files: PublicationFiles::new(root)?,
        })
    }
    fn relative(id: &str) -> Result<String> {
        if !crate::observability::trace_id(id) {
            return Err(RuntimeError::invalid("Invalid trace identifier"));
        }
        Ok(format!("memory-bank/local/retrieval-traces/{id}.json"))
    }
    pub fn save(&mut self, id: &str, bytes: &[u8], limit: usize) -> Result<()> {
        if bytes.len() > limit {
            return Err(RuntimeError::invalid("Trace byte limit exceeded"));
        }
        let path = Self::relative(id)?;
        let (parent, name) = self.files.parent(&path, true)?;
        let temporary = format!(".{id}.tmp");
        let mut file = native::file(&parent, &temporary, true)?;
        let result = (|| -> Result<()> {
            file.write_all(bytes)?;
            file.flush()?;
            file.sync_all()?;
            close_files(vec![file])?;
            self.files.check()?;
            native::rename(&parent, &temporary, &name, false)?;
            self.files.check()
        })();
        let cleanup = native::unlink(&parent, &temporary);
        if result.is_ok() {
            match cleanup {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        let parent_closed = close_files(vec![parent]);
        match result {
            Err(error) => Err(error),
            Ok(()) => parent_closed,
        }
    }
    pub fn close(mut self) -> Result<()> {
        let mut owned = Vec::new();
        owned.extend(std::mem::take(&mut self.files.directories).into_values());
        for pin in std::mem::take(&mut self.files.pins) {
            owned.push(pin.parent);
            owned.push(pin.child);
        }
        close_files(owned)
    }
    pub fn read(&mut self, id: &str, limit: usize, require_owner: bool) -> Result<Option<Vec<u8>>> {
        let path = Self::relative(id)?;
        let (parent, name) = match self.files.parent(&path, false) {
            Ok(value) => value,
            Err(error) if error.kind == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut file = match native::file(&parent, &name, false) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let before = native::identity(&file)?;
        let meta = file.metadata()?;
        if !meta.is_file() || before.links != 1 || before.size > limit as u64 {
            return Err(RuntimeError::policy("Unsupported trace image"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if require_owner && meta.uid() != unsafe { libc::getuid() } {
                return Err(RuntimeError::policy("Trace owner unavailable"));
            }
        }
        #[cfg(not(unix))]
        if require_owner {
            return Err(RuntimeError::unavailable("Trace owner unavailable"));
        }
        self.files.check()?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)?;
        let current = native::file(&parent, &name, false)?;
        if bytes.len() > limit
            || bytes.len() as u64 != before.size
            || native::identity(&file)? != before
            || native::identity(&current)? != before
        {
            return Err(RuntimeError::conflict(
                "Trace changed during metadata observation",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if require_owner
                && (file.metadata()?.uid() != meta.uid() || current.metadata()?.uid() != meta.uid())
            {
                return Err(RuntimeError::conflict("Trace owner changed"));
            }
        }
        self.files.check()?;
        close_files(vec![current, file, parent])?;
        Ok(Some(bytes))
    }
    pub fn update_existing<F>(&mut self, id: &str, limit: usize, update: F) -> Result<()>
    where
        F: FnOnce(&[u8]) -> Result<Option<Vec<u8>>>,
    {
        #[cfg(not(unix))]
        {
            let _ = (id, limit, update);
            Err(RuntimeError::unavailable(
                "In-place trace finalization unavailable",
            ))
        }
        #[cfg(unix)]
        {
            use std::{
                ffi::CString,
                os::{
                    fd::{AsRawFd, FromRawFd},
                    unix::fs::MetadataExt,
                },
            };
            let path = Self::relative(id)?;
            let (parent, name) = match self.files.parent(&path, false) {
                Ok(value) => value,
                Err(error) if error.kind == ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error),
            };
            let component = CString::new(name.as_str())
                .map_err(|_| RuntimeError::invalid("Invalid trace filename"))?;
            let fd = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    component.as_ptr(),
                    libc::O_RDWR | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::NotFound {
                    return Ok(());
                }
                return Err(error.into());
            }
            let mut file = unsafe { File::from_raw_fd(fd) };
            let identity = native::identity(&file)?;
            let meta = file.metadata()?;
            if !meta.is_file() || identity.links != 1 || identity.size > limit as u64 {
                return Err(RuntimeError::policy("Unsupported render trace image"));
            }
            self.files.check()?;
            let mut previous = Vec::new();
            Read::by_ref(&mut file)
                .take(limit as u64 + 1)
                .read_to_end(&mut previous)?;
            if previous.len() > limit
                || previous.len() as u64 != identity.size
                || native::identity(&file)? != identity
            {
                return Err(RuntimeError::conflict("Render trace changed during read"));
            }
            let Some(bytes) = update(&previous)? else {
                return Ok(());
            };
            if bytes.len() > limit {
                return Err(RuntimeError::invalid("Render trace byte limit exceeded"));
            }
            // A removed name is never created again; all writes below address
            // only this opened file, even if retention prunes it after this check.
            let named = match native::file(&parent, &name, false) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error.into()),
            };
            if native::identity(&named)? != identity || native::identity(&file)? != identity {
                return Err(RuntimeError::conflict(
                    "Render trace changed before finalization",
                ));
            }
            self.files.check()?;
            file.seek(SeekFrom::Start(0))?;
            file.write_all(&bytes)?;
            file.set_len(bytes.len() as u64)?;
            file.flush()?;
            let times = [
                libc::timespec {
                    tv_sec: meta.atime() as libc::time_t,
                    tv_nsec: meta.atime_nsec() as libc::c_long,
                },
                libc::timespec {
                    tv_sec: meta.mtime() as libc::time_t,
                    tv_nsec: meta.mtime_nsec() as libc::c_long,
                },
            ];
            if unsafe { libc::futimens(file.as_raw_fd(), times.as_ptr()) } < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            self.files.check()?;
            close_files(vec![named, file, parent])
        }
    }
    pub fn prune(&mut self, current: &str, retention: usize) -> Result<()> {
        let path = Self::relative(current)?;
        let (parent, _) = self.files.parent(&path, false)?;
        let mut files = vec![];
        for name in cursor_names(&parent, None)? {
            if !name
                .strip_suffix(".json")
                .is_some_and(crate::observability::trace_id)
            {
                continue;
            }
            let file = match native::file(&parent, &name, false) {
                Ok(file) => file,
                Err(_) => continue,
            };
            if file.metadata().is_ok_and(|meta| meta.is_file())
                && let Ok(identity) = native::identity(&file)
            {
                files.push((identity.modified, name, identity));
            }
        }
        let current_name = format!("{current}.json");
        if !files.iter().any(|(_, name, _)| name == &current_name) {
            let file = native::file(&parent, &current_name, false)?;
            if file.metadata()?.is_file() {
                let identity = native::identity(&file)?;
                files.push((identity.modified, current_name.clone(), identity));
            }
        }
        files.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let count = files.len().saturating_sub(retention);
        for (_, name, expected) in files
            .into_iter()
            .filter(|(_, name, _)| name != &current_name)
            .take(count)
        {
            self.files.check()?;
            if native::file(&parent, &name, false)
                .ok()
                .and_then(|file| native::identity(&file).ok())
                .as_ref()
                == Some(&expected)
            {
                let _ = native::unlink(&parent, &name);
            }
        }
        self.files.check()?;
        close_files(vec![parent])
    }
}

fn close_files(files: Vec<File>) -> Result<()> {
    let mut first = None;
    for file in files {
        #[cfg(unix)]
        let result = {
            use std::os::fd::IntoRawFd;
            let descriptor = file.into_raw_fd();
            if unsafe { libc::close(descriptor) } < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        };
        #[cfg(not(unix))]
        let result: std::io::Result<()> = {
            drop(file);
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Trace descriptor closure unavailable",
            ))
        };
        if let Err(error) = result
            && first.is_none()
        {
            first = Some(error);
        }
    }
    first.map(|error| Err(error.into())).unwrap_or(Ok(()))
}
