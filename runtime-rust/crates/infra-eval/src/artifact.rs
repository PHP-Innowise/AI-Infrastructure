use crate::{Result, RuntimeError};
use serde_json::Value;
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::path::Component;
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};
pub const MAX_FILE: usize = 4 * 1024 * 1024;
pub const MAX_TREE: usize = 64 * 1024 * 1024;
pub fn sha(raw: &[u8]) -> String {
    Sha256::digest(raw)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn digest(value: &Value) -> Result<String> {
    Ok(sha(infra_core::domain::json::canonical(value)?.as_bytes()))
}
pub fn spaced_digest(value: &Value, sort: bool, ascii: bool) -> Result<String> {
    Ok(sha(infra_core::domain::json::python_default(
        value, sort, ascii, false,
    )?
    .as_bytes()))
}
pub fn hex(value: &str, n: usize) -> bool {
    value.len() == n
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn exact(value: &Value, keys: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|m| m.len() == keys.len() && keys.iter().all(|k| m.contains_key(*k)))
}
pub fn invalid(message: &str) -> RuntimeError {
    RuntimeError::invalid(message)
}
pub fn strict(raw: &[u8]) -> Result<Value> {
    // Callers retain their own byte limits; lexical decoding keeps finite numbers
    // and literal reserved-key objects distinct under serde feature unification.
    infra_core::strict_json::parse(raw, raw.len(), 128, usize::MAX)
}
pub fn ordinary(path: &Path) -> Result<PathBuf> {
    let path = infra_core::fs::absolute(path)?;
    for parent in path.ancestors() {
        if let Ok(m) = fs::symlink_metadata(parent)
            && infra_core::fs::linked(&m)
        {
            return Err(invalid("linked path refused"));
        }
    }
    Ok(path)
}
pub fn overlaps(a: &Path, b: &Path) -> Result<bool> {
    let a = infra_core::fs::canonical_or_missing(a)?;
    let b = infra_core::fs::canonical_or_missing(b)?;
    Ok(a.starts_with(&b) || b.starts_with(&a))
}
pub fn relative(value: &str) -> Result<PathBuf> {
    if value.is_empty()
        || value.starts_with('/')
        || value.contains(['\\', ':', '\0'])
        || value
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(invalid("unsafe relative path"));
    }
    Ok(PathBuf::from(value))
}
/// Exact serialized spelling of native relative components; never replace invalid bytes.
pub(crate) fn snapshot_name(path: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        let std::path::Component::Normal(part) = component else {
            return Err(invalid("unsafe snapshot relative path"));
        };
        parts.push(
            part.to_str()
                .ok_or_else(|| invalid("snapshot path is not UTF-8"))?,
        );
    }
    if parts.is_empty() {
        return Err(invalid("unsafe snapshot relative path"));
    }
    Ok(parts.join("/"))
}
// This spelling is used only by the pre-existing private exclusions. It is
// never serialized, looked up or admitted as an ImageFile path.
fn snapshot_policy_name(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}
pub(crate) fn native_relative(value: &str) -> Result<PathBuf> {
    #[cfg(not(unix))]
    {
        relative(value)
    }
    #[cfg(unix)]
    {
        if value.is_empty()
            || value.starts_with('/')
            || value.contains('\0')
            || value
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(invalid("unsafe native relative path"));
        }
        let path = PathBuf::from(value);
        if path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(invalid("unsafe native relative path"));
        }
        Ok(path)
    }
}

pub fn public_text(text: &str) -> Result<()> {
    if infra_core::domain::sensitive_label(text)?.is_some() {
        return Err(invalid("sensitive input refused"));
    }
    Ok(())
}
pub fn source_allowed(name: &str) -> bool {
    let parts = name.split('/').map(str::to_lowercase).collect::<Vec<_>>();
    if parts.iter().any(|p| {
        p.starts_with(".env")
            || [
                ".git",
                ".ssh",
                ".aws",
                ".kube",
                ".venv",
                "__pycache__",
                "node_modules",
                "vendor",
                ".context-handoff",
                ".superpowers",
                ".worktrees",
                "secrets",
                ".secrets",
                "credentials",
            ]
            .contains(&p.as_str())
    }) {
        return false;
    }
    if parts.last().is_some_and(|p| {
        [
            "credentials.json",
            "auth.json",
            "settings.local.json",
            "accelerator-attach.json",
            ".mcp.json",
        ]
        .contains(&p.as_str())
    }) {
        return false;
    }
    let suffix = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    ![
        "pem", "key", "p12", "pfx", "jks", "keystore", "sqlite", "db",
    ]
    .contains(&suffix.as_str())
        && ![
            "memory-bank/local/",
            "project-brain/local/",
            "launch/",
            ".claude/worktrees/",
        ]
        .iter()
        .any(|p| name.starts_with(p))
}
#[cfg(unix)]
fn same(a: &libc::stat, b: &libc::stat) -> bool {
    a.st_dev == b.st_dev
        && a.st_ino == b.st_ino
        && a.st_size == b.st_size
        && a.st_mode == b.st_mode
        && a.st_nlink == b.st_nlink
        && a.st_mtime == b.st_mtime
        && a.st_mtime_nsec == b.st_mtime_nsec
        && a.st_ctime == b.st_ctime
        && a.st_ctime_nsec == b.st_ctime_nsec
}
#[cfg(unix)]
pub fn read_file(path: &Path, limit: usize) -> Result<(Vec<u8>, u32)> {
    use std::ffi::CString;
    use std::os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::ffi::OsStrExt,
    };
    let path = ordinary(path)?;
    let mut parent = File::open("/")?;
    let components = path
        .components()
        .filter_map(|c| {
            if let Component::Normal(p) = c {
                Some(p)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let Some(name) = components.last() else {
        return Err(invalid("regular file required"));
    };
    for part in &components[..components.len() - 1] {
        let c = CString::new(part.as_bytes()).map_err(|_| invalid("invalid path"))?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                c.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        parent = unsafe { File::from_raw_fd(fd) };
    }
    let c = CString::new(name.as_bytes()).map_err(|_| invalid("invalid path"))?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            c.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut a = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(fd.as_raw_fd(), a.as_mut_ptr()) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let a = unsafe { a.assume_init() };
    if a.st_mode & libc::S_IFMT != libc::S_IFREG
        || a.st_nlink != 1
        || a.st_size < 0
        || a.st_size as u64 > limit as u64
    {
        return Err(invalid("unsafe or oversized file"));
    }
    let mut file = File::from(fd);
    let mut raw = vec![];
    (&mut file).take(limit as u64 + 1).read_to_end(&mut raw)?;
    let mut b = std::mem::MaybeUninit::<libc::stat>::uninit();
    let mut n = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(file.as_raw_fd(), b.as_mut_ptr()) } < 0
        || unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                c.as_ptr(),
                n.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } < 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    if raw.len() > limit
        || !same(&a, &unsafe { b.assume_init() })
        || !same(&a, &unsafe { n.assume_init() })
    {
        return Err(invalid("file changed while reading"));
    }
    Ok((raw, a.st_mode & 0o777))
}
#[cfg(not(unix))]
pub fn read_file(path: &Path, limit: usize) -> Result<(Vec<u8>, u32)> {
    let path = ordinary(path)?;
    let before = fs::symlink_metadata(&path)?;
    if !before.is_file() || before.len() > limit as u64 {
        return Err(invalid("unsafe or oversized file"));
    }
    let mut f = File::open(&path)?;
    let mut raw = vec![];
    (&mut f).take(limit as u64 + 1).read_to_end(&mut raw)?;
    let after = fs::symlink_metadata(path)?;
    if raw.len() > limit
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || before.created().ok() != after.created().ok()
    {
        return Err(invalid("file changed while reading"));
    }
    Ok((raw, 0o644))
}
/// Preserve artifact path, link-count and size admission without opening an
/// ordinary descriptor for a SQLite file. SQLite owns all data descriptors so
/// closing this preflight cannot release another connection's POSIX locks.
#[cfg(unix)]
pub fn open_database_read_only(
    path: &Path,
    limit: usize,
    timeout: std::time::Duration,
) -> Result<infra_core::Store> {
    use std::ffi::CString;
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    };

    fn metadata_at(directory: i32, name: &std::ffi::CStr) -> Result<libc::stat> {
        let mut value = std::mem::MaybeUninit::<libc::stat>::uninit();
        // This inspects the name without opening the regular file. In
        // particular, no ordinary SQLite descriptor can subsequently close.
        if unsafe {
            libc::fstatat(
                directory,
                name.as_ptr(),
                value.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } < 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(unsafe { value.assume_init() })
    }

    let path = ordinary(path)?;
    let mut parent = File::open("/")?;
    let components = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect::<Vec<_>>();
    let Some(name) = components.last() else {
        return Err(invalid("regular database required"));
    };
    for part in &components[..components.len() - 1] {
        let name = CString::new(part.as_bytes()).map_err(|_| invalid("invalid database path"))?;
        let raw = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if raw < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        // Only directory handles are opened by this admission layer.
        parent = unsafe { File::from_raw_fd(raw) };
    }
    let name = CString::new(name.as_bytes()).map_err(|_| invalid("invalid database path"))?;
    let before = metadata_at(parent.as_raw_fd(), &name)?;
    if before.st_mode & libc::S_IFMT != libc::S_IFREG
        || before.st_nlink != 1
        || before.st_size < 0
        || before.st_size as u64 > limit as u64
    {
        return Err(invalid("unsafe or oversized database"));
    }
    let store = infra_core::Store::open_read_only(&path, timeout)?;
    let after = metadata_at(parent.as_raw_fd(), &name)?;
    ordinary(&path)?;
    let absolute =
        CString::new(path.as_os_str().as_bytes()).map_err(|_| invalid("invalid database path"))?;
    let named = metadata_at(libc::AT_FDCWD, &absolute)?;
    if !same(&before, &after) || !same(&before, &named) {
        return Err(invalid("database changed while opening"));
    }
    Ok(store)
}
#[cfg(not(unix))]
pub fn open_database_read_only(
    path: &Path,
    limit: usize,
    timeout: std::time::Duration,
) -> Result<infra_core::Store> {
    let path = ordinary(path)?;
    // native_image uses an attributes-only Windows handle, never a content
    // read. Keep the artifact's strict all-reparse refusal at both boundaries.
    let before =
        infra_core::fs::native_image(&path)?.ok_or_else(|| invalid("database unavailable"))?;
    if before.kind & 0o170000 != 0o100000
        || before.links != 1
        || before.reparse_tag != 0
        || before.attributes & 0x400 != 0
        || before.size > limit as u64
    {
        return Err(invalid("unsafe or oversized database"));
    }
    let store = infra_core::Store::open_read_only(&path, timeout)?;
    ordinary(&path)?;
    if infra_core::fs::native_image(&path)?.as_ref() != Some(&before) {
        return Err(invalid("database changed while opening"));
    }
    Ok(store)
}
pub fn read_json(path: &Path, limit: usize) -> Result<Value> {
    strict(&read_file(path, limit)?.0)
}
pub fn mode(path: &Path) -> Result<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Ok(fs::symlink_metadata(path)?.permissions().mode() & 0o7777)
    }
    #[cfg(not(unix))]
    {
        Ok(if fs::metadata(path)?.permissions().readonly() {
            0o444
        } else {
            0o644
        })
    }
}
pub fn chmod(path: &Path, bits: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(bits))?;
    }
    #[cfg(not(unix))]
    {
        let mut p = fs::metadata(path)?.permissions();
        p.set_readonly(bits & 0o200 == 0);
        fs::set_permissions(path, p)?;
    }
    Ok(())
}
pub fn private_dir(path: &Path) -> Result<()> {
    use infra_core::bank::publication::{PublicationFiles, private::RootProtection};
    let path = ordinary(path)?;
    let owner = PublicationFiles::open_directory_root(&path, RootProtection::SelectedRoot)?;
    owner.check()
}
pub fn write_json(path: &Path, value: &Value) -> Result<()> {
    ordinary(path)?;
    let mut raw =
        infra_core::domain::json::serialize(value, true, false, true, false)?.into_bytes();
    raw.push(b'\n');
    if raw.len() > MAX_TREE {
        return Err(invalid("artifact byte limit"));
    }
    infra_core::fs::atomic_write(path, &raw)?;
    chmod(path, 0o600)
}
pub fn fresh_file(path: &Path, raw: &[u8], bits: u32) -> Result<()> {
    ordinary(path)?;
    if let Some(p) = path.parent() {
        private_dir(p)?;
    }
    let mut o = OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut f = o.open(path)?;
    f.write_all(raw)?;
    chmod(path, bits)
}
#[derive(Clone, Debug)]
pub struct ImageFile {
    pub path: String,
    pub bytes: Vec<u8>,
    pub mode: u32,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Public,
    NativePublic,
    Physical,
    Candidate,
    Oracle,
}
pub fn image(root: &Path, kind: ImageKind, deadline: Option<Instant>) -> Result<Vec<ImageFile>> {
    image_inner(root, kind, deadline, None)
}
pub(crate) fn image_with_canonical_engine(
    root: &Path,
    kind: ImageKind,
    deadline: Option<Instant>,
    engine: &crate::executable::CanonicalEngine<'_>,
) -> Result<Vec<ImageFile>> {
    if !matches!(
        kind,
        ImageKind::Public | ImageKind::NativePublic | ImageKind::Physical
    ) {
        return Err(invalid(
            "canonical engine authority is unavailable for this snapshot kind",
        ));
    }
    image_inner(root, kind, deadline, Some(engine))
}
fn image_inner(
    root: &Path,
    kind: ImageKind,
    deadline: Option<Instant>,
    engine: Option<&crate::executable::CanonicalEngine<'_>>,
) -> Result<Vec<ImageFile>> {
    let root = ordinary(root)?;
    if !root.is_dir() {
        return Err(invalid("ordinary directory required"));
    }
    let mut out = vec![];
    let (mut total, mut visited) = (0usize, 0usize);
    #[expect(
        clippy::too_many_arguments,
        reason = "Retain snapshot limits while carrying explicit engine authority through traversal"
    )]
    fn walk(
        root: &Path,
        dir: &Path,
        kind: ImageKind,
        deadline: Option<Instant>,
        engine: Option<&crate::executable::CanonicalEngine<'_>>,
        out: &mut Vec<ImageFile>,
        total: &mut usize,
        visited: &mut usize,
    ) -> Result<()> {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            return Err(invalid("collection deadline"));
        }
        let mut entries = fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|e| e.file_name());
        *visited += entries.len() + 1;
        if *visited > 100_000 {
            return Err(invalid("snapshot scan limit"));
        }
        for entry in entries {
            let path = entry.path();
            let relative_path = path
                .strip_prefix(root)
                .map_err(|_| invalid("snapshot root"))?;
            let name = snapshot_policy_name(relative_path);
            let m = fs::symlink_metadata(&path)?;
            if ((kind == ImageKind::Public || kind == ImageKind::NativePublic)
                && !source_allowed(&name))
                || (kind == ImageKind::NativePublic
                    && name.split('/').any(|p| {
                        [
                            ".codex", ".Codex", ".claude", ".cursor", ".agents", ".cache",
                        ]
                        .contains(&p)
                    }))
                || (kind == ImageKind::Candidate
                    && name.split('/').any(|p| p == ".git" || p == "__pycache__"))
            {
                continue;
            }
            if infra_core::fs::linked(&m) {
                return Err(invalid("snapshot link refused"));
            }
            if m.is_dir() {
                if engine.is_some() && crate::executable::is_engine_slot(&name) {
                    return Err(invalid("snapshot engine slot is not an ordinary file"));
                }
                walk(root, &path, kind, deadline, engine, out, total, visited)?;
            } else if m.is_file() {
                let name = snapshot_name(relative_path)?;
                if matches!(kind, ImageKind::Public | ImageKind::NativePublic) {
                    relative(&name)?;
                }
                let native_engine = [
                    "memory-bank/bin/infra-engine",
                    "memory-bank/bin/infra-engine.exe",
                ]
                .contains(&name.as_str());
                let admitted_engine = engine.filter(|authority| authority.path() == name);
                if engine.is_some() && native_engine && admitted_engine.is_none() {
                    return Err(invalid(
                        "snapshot contains an unapproved native engine slot",
                    ));
                }
                let limit = if admitted_engine.is_some()
                    || kind == ImageKind::Candidate
                    || kind == ImageKind::Oracle
                    || engine.is_none() && native_engine
                {
                    512 * 1024 * 1024
                } else {
                    MAX_FILE
                };
                let (raw, mode) = read_file(&path, limit)?;
                if native_engine
                    && !(raw.starts_with(b"\x7fELF")
                        || raw.starts_with(b"MZ")
                        || raw.starts_with(&[0xcf, 0xfa, 0xed, 0xfe])
                        || raw.starts_with(&[0xfe, 0xed, 0xfa, 0xcf]))
                {
                    return Err(invalid(
                        "installed native engine has invalid executable image",
                    ));
                }
                if let Some(authority) = admitted_engine {
                    if !authority.matches(&name, &raw, mode) {
                        return Err(invalid(
                            "snapshot native engine differs from canonical authority",
                        ));
                    }
                } else {
                    *total = (*total)
                        .checked_add(raw.len())
                        .ok_or_else(|| invalid("snapshot byte bound overflow"))?;
                }
                let cap = if kind == ImageKind::Candidate || kind == ImageKind::Oracle {
                    512 * 1024 * 1024
                } else {
                    MAX_TREE
                };
                if *total > cap || out.len() >= if cap == MAX_TREE { 20_000 } else { 100_000 } {
                    return Err(invalid("snapshot byte/file bound"));
                }
                out.push(ImageFile {
                    path: name,
                    bytes: raw,
                    mode,
                });
            } else {
                return Err(invalid("snapshot special file refused"));
            }
        }
        Ok(())
    }
    walk(
        &root,
        &root,
        kind,
        deadline,
        engine,
        &mut out,
        &mut total,
        &mut visited,
    )?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}
pub fn image_digest(rows: &[ImageFile], spaced: bool) -> Result<String> {
    let value = Value::Array(
        rows.iter()
            .map(|f| serde_json::json!([f.path, sha(&f.bytes), f.mode]))
            .collect(),
    );
    if spaced {
        spaced_digest(&value, true, false)
    } else {
        digest(&value)
    }
}
pub fn candidate_digest(root: &Path) -> Result<String> {
    digest(&Value::Array(
        image(root, ImageKind::Candidate, None)?
            .iter()
            .map(|f| serde_json::json!([f.path, sha(&f.bytes)]))
            .collect(),
    ))
}

struct NativeCopyTarget {
    path: PathBuf,
    owner: infra_core::bank::publication::PublicationFiles,
    root: infra_core::bank::publication::private::CreatedNativeDirectory,
    directories: std::collections::BTreeMap<
        PathBuf,
        infra_core::bank::publication::private::CreatedNativeDirectory,
    >,
}
impl NativeCopyTarget {
    fn create(target: &Path) -> Result<Self> {
        use infra_core::bank::publication::{PublicationFiles, private::RootProtection};
        if fs::symlink_metadata(target).is_ok() {
            return Err(invalid("fresh target required"));
        }
        let path = ordinary(target)?;
        let parent = path
            .parent()
            .ok_or_else(|| invalid("copy parent unavailable"))?;
        let name = path
            .file_name()
            .ok_or_else(|| invalid("copy target needs one native leaf"))?;
        let spelling = name
            .to_str()
            .ok_or_else(|| invalid("copy target name is not UTF-8"))?;
        if native_relative(spelling)?.components().count() != 1 {
            return Err(invalid("copy target needs one native leaf"));
        }
        let mut owner =
            PublicationFiles::open_directory_root(parent, RootProtection::CreatedDirectories)?;
        let root = owner.create_private_native_directory_capability(name)?;
        let result = Self {
            path,
            owner,
            root,
            directories: std::collections::BTreeMap::new(),
        };
        result.check()?;
        Ok(result)
    }
    fn check(&self) -> Result<()> {
        self.owner.check()?;
        self.root.check()
    }
    fn check_directories(&self) -> Result<()> {
        self.check()?;
        for directory in self.directories.values() {
            directory.check()?;
        }
        Ok(())
    }
    fn check_chain(&self, relative: &Path) -> Result<()> {
        self.check()?;
        let mut key = PathBuf::new();
        for component in relative.components() {
            let std::path::Component::Normal(name) = component else {
                return Err(invalid("unsafe native copy directory"));
            };
            key.push(name);
            self.directories
                .get(&key)
                .ok_or_else(|| invalid("native copy directory was not admitted"))?
                .check()?;
        }
        Ok(())
    }
    fn ensure_directory(&mut self, relative: &Path) -> Result<()> {
        self.check()?;
        let mut key = PathBuf::new();
        let mut parent = PathBuf::new();
        for component in relative.components() {
            let std::path::Component::Normal(name) = component else {
                return Err(invalid("unsafe native copy directory"));
            };
            key.push(name);
            if !self.directories.contains_key(&key) {
                let directory = if parent.as_os_str().is_empty() {
                    &self.root
                } else {
                    self.directories
                        .get(&parent)
                        .ok_or_else(|| invalid("native copy parent unavailable"))?
                };
                let created = directory.create_private_child(name)?;
                self.directories.insert(key.clone(), created);
            }
            self.directories.get(&key).unwrap().check()?;
            parent = key.clone();
            self.check()?;
        }
        Ok(())
    }
    fn files(&mut self, rows: &[ImageFile]) -> Result<()> {
        let mut names = BTreeSet::new();
        for row in rows {
            self.check()?;
            let name = native_relative(&row.path)?;
            if !names.insert(&row.path) {
                return Err(invalid("duplicate image file"));
            }
            if let Some(parent) = name.parent() {
                self.ensure_directory(parent)?;
            }
            fresh_file(&self.path.join(&name), &row.bytes, row.mode)?;
            if let Some(parent) = name.parent() {
                self.check_chain(parent)?;
            }
            self.check()?;
        }
        self.check()
    }
    fn set_directory_mode(&mut self, relative: &Path, mode: u32) -> Result<()> {
        self.check()?;
        if relative.as_os_str().is_empty() {
            self.owner
                .set_created_native_directory_mode(&mut self.root, mode)?;
        } else {
            self.directories
                .get_mut(relative)
                .ok_or_else(|| invalid("native directory mode was not admitted"))?
                .set_mode(mode)?;
        }
        self.check()
    }
    fn cleanup(&mut self) -> Result<()> {
        // A replaced root/created directory is a refusal, never permission to
        // repair modes on or remove the replacement. This is bounded custody,
        // not a claim of atomic whole-tree CAS against concurrent namespace edits.
        self.check_directories()?;
        self.set_directory_mode(Path::new(""), 0o700)?;
        let mut names = self.directories.keys().cloned().collect::<Vec<_>>();
        names.sort_by_key(|name| name.components().count());
        for name in names {
            self.set_directory_mode(&name, 0o700)?;
        }
        self.check_directories()?;
        fs::remove_dir_all(&self.path)?;
        Ok(())
    }
}
pub fn copy_image(rows: &[ImageFile], target: &Path) -> Result<()> {
    copy_image_policy(rows, target, false)
}
pub(crate) fn copy_native_image(rows: &[ImageFile], target: &Path) -> Result<()> {
    copy_image_policy(rows, target, true)
}
/// Native corpus/candidate copies preserve the directory namespace and ordinary
/// modes as well as the held file bytes. Public memory snapshots intentionally
/// retain their separate file-only portable profile.
pub(crate) fn copy_native_tree(
    source: &Path,
    target: &Path,
    kind: ImageKind,
    engine: Option<&crate::executable::CanonicalEngine<'_>>,
) -> Result<()> {
    copy_native_tree_using(source, target, kind, engine, |_| Ok(()))
}
fn copy_native_tree_using(
    source: &Path,
    target: &Path,
    kind: ImageKind,
    engine: Option<&crate::executable::CanonicalEngine<'_>>,
    after_files: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    if !matches!(kind, ImageKind::Physical | ImageKind::Candidate)
        || engine.is_some() && kind != ImageKind::Physical
    {
        return Err(invalid("native tree copy profile is unavailable"));
    }
    let source = ordinary(source)?;
    if overlaps(&source, target)? {
        return Err(invalid("native tree copy paths overlap"));
    }
    let files = if let Some(engine) = engine {
        image_with_canonical_engine(&source, kind, None, engine)?
    } else {
        image(&source, kind, None)?
    };
    let root_mode = mode(&source)?;
    fn directories(
        root: &Path,
        at: &Path,
        kind: ImageKind,
        output: &mut Vec<(PathBuf, u32)>,
        visited: &mut usize,
    ) -> Result<()> {
        let mut entries = fs::read_dir(at)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        *visited += entries.len() + 1;
        if *visited > 100_000 {
            return Err(invalid("native tree directory scan limit"));
        }
        for entry in entries {
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|_| invalid("snapshot root"))?;
            let policy = snapshot_policy_name(relative);
            if kind == ImageKind::Candidate
                && policy
                    .split('/')
                    .any(|part| part == ".git" || part == "__pycache__")
            {
                continue;
            }
            let metadata = fs::symlink_metadata(&path)?;
            if infra_core::fs::linked(&metadata) {
                return Err(invalid("native tree link refused"));
            }
            if metadata.is_dir() {
                if relative
                    .components()
                    .any(|part| !matches!(part, std::path::Component::Normal(_)))
                {
                    return Err(invalid("unsafe native directory path"));
                }
                // Copy-only metadata remains native. Any serialized directory
                // proof uses the separate strict-UTF8 snapshot_name boundary.
                output.push((relative.to_path_buf(), mode(&path)?));
                directories(root, &path, kind, output, visited)?;
            } else if !metadata.is_file() {
                return Err(invalid("native tree special file refused"));
            }
        }
        Ok(())
    }
    let mut folders = Vec::new();
    directories(&source, &source, kind, &mut folders, &mut 0)?;
    let mut target = NativeCopyTarget::create(target)?;
    let result = (|| {
        target.files(&files)?;
        after_files(&target.path)?;
        target.check_directories()?;
        for (name, _) in &folders {
            target.ensure_directory(name)?;
        }
        // Preserve native directory names and restore deepest children first.
        for (name, bits) in folders.iter().rev() {
            target.set_directory_mode(name, *bits)?;
        }
        target.set_directory_mode(Path::new(""), root_mode)?;
        target.check_directories()
    })();
    if result.is_err() {
        let _ = target.cleanup();
    }
    result
}

fn copy_image_policy(rows: &[ImageFile], target: &Path, native: bool) -> Result<()> {
    if native {
        let mut target = NativeCopyTarget::create(target)?;
        let result = target.files(rows).and_then(|()| target.check_directories());
        if result.is_err() {
            let _ = target.cleanup();
        }
        return result;
    }
    if target.exists() {
        return Err(invalid("fresh target required"));
    }
    private_dir(target)?;
    let result = (|| {
        let mut names = BTreeSet::new();
        for file in rows {
            let name = relative(&file.path)?;
            if !names.insert(&file.path) {
                return Err(invalid("duplicate image file"));
            }
            fresh_file(&target.join(name), &file.bytes, file.mode)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(target);
    }
    result
}
pub struct Scratch(pub PathBuf);
impl Scratch {
    pub fn new(parent: &Path, prefix: &str) -> Result<Self> {
        Self::create(
            parent,
            &format!("{prefix}-{}", uuid::Uuid::new_v4().simple()),
        )
    }
    fn create(parent: &Path, name: &str) -> Result<Self> {
        use infra_core::bank::publication::{PublicationFiles, private::RootProtection};
        let leaf = relative(name)?;
        if leaf.components().count() != 1 {
            return Err(invalid("scratch directory needs one ordinary leaf name"));
        }
        let parent = ordinary(parent)?;
        let mut owner =
            PublicationFiles::open_directory_root(&parent, RootProtection::CreatedDirectories)?;
        owner.create_private_directory_exclusive(name)?;
        owner.check()?;
        Ok(Self(parent.join(leaf)))
    }
}

#[cfg(test)]
#[path = "artifact_scratch_tests.rs"]
mod scratch_tests;
impl Drop for Scratch {
    fn drop(&mut self) {
        // A retained native custody owner may still need this work. A fatal
        // loss of cleanup proof never turns into automatic state deletion.
        if !crate::process::custody_lost() {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
pub struct ExclusiveLock {
    path: PathBuf,
    _file: File,
}
impl ExclusiveLock {
    pub fn acquire(path: &Path) -> Result<Self> {
        let f = OpenOptions::new().write(true).create_new(true).open(path)?;
        chmod(path, 0o600)?;
        Ok(Self {
            path: path.into(),
            _file: f,
        })
    }
}
impl Drop for ExclusiveLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn strip_tail<'a>(mut value: &'a str, suffix: &str) -> &'a str {
    if suffix.is_empty() {
        return value;
    }
    while let Some(prefix) = value.strip_suffix(suffix) {
        value = prefix;
    }
    value
}
pub fn strip_trailing_space(value: &str) -> &str {
    let mut end = value.len();
    for (index, character) in value.char_indices().rev() {
        if character.is_whitespace() {
            end = index;
        } else {
            break;
        }
    }
    &value[..end]
}

#[cfg(test)]
#[path = "artifact_image_tests.rs"]
mod image_name_contract_tests;
