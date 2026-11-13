use crate::{Result, RuntimeError};
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    marker::PhantomData,
    path::{Component, Path, PathBuf},
    rc::Rc,
    sync::Arc,
};
use uuid::Uuid;

pub type FileSnapshot = BTreeMap<PathBuf, Option<Vec<u8>>>;

mod python_paths;
mod spelling;
pub use python_paths::python_realpath;
pub use python_paths::python_realpath_spelling;
pub use spelling::{canonicalize, plain_windows_path};

pub fn linked(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Conservatively refuse redirecting/unknown reparse authority paths.
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

/// Absolute lexical spelling; missing tail components need not exist.
pub fn absolute(path: &Path) -> Result<PathBuf> {
    let input = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut result = PathBuf::new();
    for part in input.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            _ => result.push(part.as_os_str()),
        }
    }
    Ok(result)
}

pub fn canonical_or_missing(path: &Path) -> Result<PathBuf> {
    let mut current = absolute(path)?;
    let mut missing = Vec::new();
    loop {
        match canonicalize(&current) {
            Ok(mut base) => {
                for item in missing.iter().rev() {
                    base.push(item);
                }
                return Ok(base);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = current
                    .file_name()
                    .ok_or_else(|| RuntimeError::unavailable("Cannot resolve filesystem root"))?
                    .to_os_string();
                missing.push(name);
                if !current.pop() {
                    return Err(RuntimeError::unavailable("Cannot resolve filesystem root"));
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
}

pub fn admit_directory(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !linked_at(path, &metadata)? => Ok(true),
        Ok(_) => Err(RuntimeError::policy(
            "Authority directory is linked or not a directory",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub fn ensure_directory(path: &Path, private: bool) -> Result<()> {
    if admit_directory(path)? {
        return Ok(());
    }
    #[cfg(windows)]
    if private && !path.as_os_str().is_empty() {
        use crate::bank::publication::{PublicationFiles, private::RootProtection};
        // Existing directories keep their permissions. Missing components need
        // a private security descriptor at creation, before any state is written.
        let files =
            PublicationFiles::open_directory_root(path, RootProtection::CreatedDirectories)?;
        return files.check();
    }
    if let Some(parent) = path.parent()
        && parent != path
    {
        ensure_directory(parent, private)?;
    }
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    let mut builder = builder;
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(if private { 0o700 } else { 0o777 });
    }
    match builder.create(path) {
        Ok(()) => {
            #[cfg(unix)]
            if private {
                use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
                let file = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open(path)?;
                file.set_permissions(std::fs::Permissions::from_mode(0o700))?;
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => admit_directory(path)?
            .then_some(())
            .ok_or_else(|| RuntimeError::unavailable("Directory disappeared during creation")),
        Err(error) => Err(error.into()),
    }
}

/// Reject linked authority components from an already admitted anchor down.
pub fn admit_descendant(anchor: &Path, path: &Path) -> Result<()> {
    let suffix = path
        .strip_prefix(anchor)
        .map_err(|_| RuntimeError::policy("Authority path escapes its anchor"))?;
    admit_directory(anchor)?;
    let mut current = anchor.to_path_buf();
    for part in suffix.components() {
        if !matches!(part, Component::Normal(_)) {
            return Err(RuntimeError::policy("Invalid authority path component"));
        }
        current.push(part.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if linked_at(&current, &metadata)? => {
                return Err(RuntimeError::policy("Linked authority path is refused"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn regular_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    options
}

pub fn read_regular(path: &Path) -> Result<Vec<u8>> {
    let observed = fs::symlink_metadata(path)?;
    if linked_at(path, &observed)? || !observed.is_file() {
        return Err(RuntimeError::policy(
            "Source is linked or not a regular file",
        ));
    }
    let before = native_identity::identity(path)?
        .ok_or_else(|| RuntimeError::unavailable("Source disappeared"))?;
    let mut file = regular_options().open(path)?;
    #[cfg(unix)]
    {
        let metadata = file.metadata()?;
        let opened = native_identity::from_metadata(&metadata);
        if before != opened || !metadata.is_file() {
            return Err(RuntimeError::conflict("Opened source identity changed"));
        }
    }
    #[cfg(not(unix))]
    {
        let opened = native_identity::handle_identity(&file)?;
        if before != opened || !file.metadata()?.is_file() {
            return Err(RuntimeError::conflict("Opened source identity changed"));
        }
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    if native_identity::handle_identity(&file)? != before
        || native_identity::identity(path)?.as_ref() != Some(&before)
    {
        return Err(RuntimeError::conflict("Source changed during read"));
    }
    Ok(bytes)
}

pub fn read_text(path: &Path) -> Result<String> {
    let text = String::from_utf8(read_regular(path)?)
        .map_err(|_| RuntimeError::invalid("Source is not UTF-8"))?;
    Ok(text.replace("\r\n", "\n").replace('\r', "\n"))
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| RuntimeError::invalid("Atomic destination has no parent"))?;
    ensure_directory(parent, false)?;
    if let Ok(image) = fs::symlink_metadata(path)
        && (linked_at(path, &image)? || !image.is_file())
    {
        return Err(RuntimeError::policy(
            "Atomic destination is not an ordinary file",
        ));
    }
    let temporary = parent.join(format!(
        ".{}.{}",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("record"),
        Uuid::new_v4()
    ));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| -> Result<()> {
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

pub fn atomic_json(path: &Path, value: &Value) -> Result<()> {
    let mut bytes = crate::domain::json::sorted_pretty(value)?.into_bytes();
    bytes.push(b'\n');
    atomic_write(path, &bytes)
}

pub fn snapshot(paths: impl IntoIterator<Item = PathBuf>) -> Result<FileSnapshot> {
    let mut result = BTreeMap::new();
    for path in paths {
        let bytes = match fs::symlink_metadata(&path) {
            Ok(_) => Some(read_regular(&path)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        result.insert(path, bytes);
    }
    Ok(result)
}

pub fn restore(images: &FileSnapshot) -> Result<()> {
    let mut failure = None;
    for (path, bytes) in images {
        let result = match bytes {
            Some(bytes) => atomic_write(path, bytes),
            None => match fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.into()),
            },
        };
        if let Err(error) = result
            && failure.is_none()
        {
            failure = Some(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

pub fn append_whole_line(path: &Path, line: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| RuntimeError::invalid("Journal has no parent"))?;
    ensure_directory(parent, false)?;
    let mut options = OpenOptions::new();
    options.create(true).append(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let mut file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(RuntimeError::policy("Journal is not a regular file"));
    }
    let previous = file.metadata()?.len();
    if let Err(error) = file.write_all(line).and_then(|_| file.sync_all()) {
        let _ = file.set_len(previous);
        return Err(error.into());
    }
    Ok(())
}

thread_local! { static HELD_LOCKS: RefCell<BTreeMap<PathBuf, (Arc<File>, usize)>> = const { RefCell::new(BTreeMap::new()) }; }

fn reuse_held_lock(key: &Path, candidate: &File) -> Result<Option<Arc<File>>> {
    let wanted = native_identity::handle_identity(candidate)?;
    if !candidate.metadata()?.is_file() || wanted.links != 1 || wanted.reparse_tag != 0 {
        return Err(RuntimeError::policy(
            "Mutation lock must have one ordinary file link",
        ));
    }
    HELD_LOCKS.with(|held| {
        let mut held = held.borrow_mut();
        let Some((file, depth)) = held.get_mut(key) else {
            return Ok(None);
        };
        let actual = native_identity::handle_identity(file)?;
        if (actual.device, actual.file, actual.kind & 0o170000)
            != (wanted.device, wanted.file, wanted.kind & 0o170000)
            || actual.links != 1
            || actual.reparse_tag != 0
        {
            return Err(RuntimeError::conflict(
                "Mutation lock identity changed at a held key",
            ));
        }
        *depth = depth
            .checked_add(1)
            .ok_or_else(|| RuntimeError::unavailable("Mutation lock reentrancy exhausted"))?;
        Ok(Some(file.clone()))
    })
}

/// Reentrant on one thread; cross-thread/process exclusion uses the OS lock.
pub struct MutationLock {
    key: PathBuf,
    _file: Arc<File>,
    _same_thread: PhantomData<Rc<()>>,
}

impl MutationLock {
    /// Acquire the native Brain lock without waiting, using an already admitted
    /// rooted handle. Cursor custody supplies and rechecks the parent chain.
    /// Reentrancy shares the existing mutation registry and OS lock with Brain.
    pub fn try_acquire_anchored(repository: &Path, file: File) -> Result<Self> {
        let key = canonical_or_missing(&repository.join("project-brain/local/runtime.lock"))?;
        if let Some(file) = reuse_held_lock(&key, &file)? {
            return Ok(Self {
                key,
                _file: file,
                _same_thread: PhantomData,
            });
        }
        if !file.metadata()?.is_file() {
            return Err(RuntimeError::policy("Cursor native lock is not regular"));
        }
        file.try_lock()
            .map_err(|_| RuntimeError::unavailable("Cursor native lock is busy or unavailable"))?;
        let file = Arc::new(file);
        HELD_LOCKS.with(|held| {
            held.borrow_mut().insert(key.clone(), (file.clone(), 1));
        });
        Ok(Self {
            key,
            _file: file,
            _same_thread: PhantomData,
        })
    }

    /// Acquire the shared mutex using a caller-admitted rooted file.
    /// The caller performs custody-aware recovery before protected mutation.
    pub fn acquire_anchored(repository: &Path, file: File) -> Result<Self> {
        let key = canonical_or_missing(&repository.join("project-brain/local/runtime.lock"))?;
        if let Some(file) = reuse_held_lock(&key, &file)? {
            return Ok(Self {
                key,
                _file: file,
                _same_thread: PhantomData,
            });
        }
        file.lock()?;
        let file = Arc::new(file);
        HELD_LOCKS.with(|held| {
            held.borrow_mut().insert(key.clone(), (file.clone(), 1));
        });
        Ok(Self {
            key,
            _file: file,
            _same_thread: PhantomData,
        })
    }
    pub fn acquire(repository: &Path) -> Result<Self> {
        let anchor = repository.join("project-brain");
        admit_directory(&anchor)?;
        let directory = anchor.join("local");
        admit_descendant(&anchor, &directory)?;
        ensure_directory(&directory, false)?;
        let path = directory.join("runtime.lock");
        admit_descendant(&anchor, &path)?;
        let key = canonical_or_missing(&path)?;
        let mut options = OpenOptions::new();
        options.create(true).append(true).read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
        }
        let file = Arc::new(options.open(path)?);
        if let Some(file) = reuse_held_lock(&key, &file)? {
            return Ok(Self {
                key,
                _file: file,
                _same_thread: PhantomData,
            });
        }
        file.lock()?;
        HELD_LOCKS.with(|held| {
            held.borrow_mut().insert(key.clone(), (file.clone(), 1));
        });
        let guard = Self {
            key,
            _file: file,
            _same_thread: PhantomData,
        };
        crate::bank::retirement::recover_pending(repository)?;
        Ok(guard)
    }
}

impl Drop for MutationLock {
    fn drop(&mut self) {
        HELD_LOCKS.with(|held| {
            let mut held = held.borrow_mut();
            let release = held
                .get_mut(&self.key)
                .map(|(_, depth)| {
                    *depth -= 1;
                    *depth == 0
                })
                .unwrap_or(false);
            if release && let Some((file, _)) = held.remove(&self.key) {
                let _ = file.unlock();
            }
        });
    }
}

pub mod native_identity;
pub use native_identity::NativeIdentity;
pub fn linked_at(path: &Path, metadata: &fs::Metadata) -> Result<bool> {
    native_identity::redirecting(path, metadata)
}
pub fn native_image(path: &Path) -> Result<Option<NativeIdentity>> {
    native_identity::identity(path)
}

#[cfg(test)]
mod anchored_lock_identity_tests {
    use super::*;
    use std::io::Write;
    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn anchored_reentrancy_shares_the_same_file_and_refuses_a_new_inode_at_the_key() {
        let root = std::env::temp_dir().join(format!("infra-anchored-lock-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let root = canonicalize(root).unwrap();
        let temp = Directory(root.clone());
        fs::create_dir_all(root.join("project-brain/local")).unwrap();
        let path = root.join("project-brain/local/runtime.lock");
        let mut initial = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        initial.write_all(b"original lock").unwrap();
        let outer = MutationLock::acquire_anchored(&root, initial).unwrap();
        let same = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let inner = MutationLock::try_acquire_anchored(&root, same).unwrap();
        drop(inner);
        fs::rename(&path, root.join("project-brain/local/retained.lock")).unwrap();
        let mut replacement = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        replacement.write_all(b"replacement must survive").unwrap();
        let error = MutationLock::try_acquire_anchored(&root, replacement.try_clone().unwrap())
            .err()
            .expect("reentrant key accepted a different lock inode");
        assert_eq!(error.kind, crate::ErrorKind::Conflict);
        let error = MutationLock::acquire_anchored(&root, replacement)
            .err()
            .expect("blocking reentrant key accepted a different lock inode");
        assert_eq!(error.kind, crate::ErrorKind::Conflict);
        assert_eq!(fs::read(&path).unwrap(), b"replacement must survive");
        drop(outer);
        drop(temp);
    }
}
