//! Rooted Creator workspace materialization from an explicitly admitted native engine.
use crate::{HarnessError, Result, store};
use infra_core::bank::publication::{FileImageIdentity, PublicationFiles};
use infra_tooling::artifacts::Artifact;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
    time::Duration,
};

const ENGINE_LIMIT: usize = 512 * 1024 * 1024;
const DATA_LIMIT: usize = 128 * 1024 * 1024;
const CREATOR_CONFIG: &str = "project-brain/config/runtime.json";
const CONFIG_LIMIT: usize = 64 * 1024;
const NAME: &str = if cfg!(windows) {
    "infra-engine.exe"
} else {
    "infra-engine"
};
const SCOPES: &[&str] = &[".agents", ".claude", ".cursor", ".codex"];

pub struct CreatorEngine {
    bytes: Arc<Vec<u8>>,
    artifact: Artifact,
    capability: Value,
    source: PublicationFiles,
    name: String,
    identity: FileImageIdentity,
    _probe: tempfile::TempDir,
}
fn native(bytes: &[u8]) -> bool {
    if cfg!(windows) {
        bytes.starts_with(b"MZ")
    } else if cfg!(target_os = "macos") {
        [
            [0xcf, 0xfa, 0xed, 0xfe],
            [0xfe, 0xed, 0xfa, 0xcf],
            [0xca, 0xfe, 0xba, 0xbe],
            [0xbe, 0xba, 0xfe, 0xca],
        ]
        .iter()
        .any(|mark| bytes.starts_with(mark))
    } else {
        bytes.starts_with(b"\x7fELF")
    }
}
fn capabilities(value: &Value, needs_creator: bool) -> Result<Vec<String>> {
    let list = value["capabilities"]
        .as_array()
        .filter(|items| items.len() <= 256)
        .ok_or_else(|| {
            HarnessError::invalid("Native engine capabilities must be bounded strings.")
        })?;
    let values = list
        .iter()
        .map(|item| {
            item.as_str()
                .filter(|text| !text.is_empty())
                .map(str::to_string)
                .ok_or_else(|| HarnessError::invalid("Invalid native capability."))
        })
        .collect::<Result<Vec<_>>>()?;
    if values.iter().collect::<BTreeSet<_>>().len() != values.len() {
        return Err(HarnessError::invalid("Duplicate native capability."));
    }
    let required =
        infra_generator::assets::runtime_contract()["native_engine"]["required_capabilities"]
            .as_array()
            .cloned()
            .unwrap_or_default();
    if (needs_creator && !values.iter().any(|item| item == "creator"))
        || required.iter().any(|item| {
            !values
                .iter()
                .any(|value| Some(value.as_str()) == item.as_str())
        })
    {
        return Err(HarnessError::invalid(
            "Selected engine lacks Creator/runtime capabilities.",
        ));
    }
    Ok(values)
}

fn probe_image_unchanged(
    held: &mut PublicationFiles,
    expected_image: FileImageIdentity,
) -> infra_core::Result<()> {
    held.check()?;
    if held.image_identity(NAME)? != Some(expected_image) {
        return Err(infra_core::RuntimeError::conflict(
            "Admitted native probe image changed.",
        ));
    }
    Ok(())
}

fn probe_timeout() -> infra_core::RuntimeError {
    infra_core::RuntimeError::coded(
        infra_core::ErrorKind::Unavailable,
        "native-timeout",
        "Native capability probe time limit exceeded",
    )
}

fn run_held_probe(
    held: &mut PublicationFiles,
    expected: &[u8],
    expected_image: FileImageIdentity,
    arguments: &[std::ffi::OsString],
    timeout: Duration,
) -> infra_core::Result<infra_tooling::process::ProcessOutput> {
    let deadline = std::time::Instant::now()
        .checked_add(timeout.min(Duration::from_secs(5)))
        .ok_or_else(probe_timeout)?;
    loop {
        probe_image_unchanged(held, expected_image)?;
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(probe_timeout());
        }
        let result = infra_tooling::process::run(
            held.root_path().join(NAME).as_os_str(),
            arguments,
            held.root_path(),
            None,
            remaining,
            256 * 1024,
        );
        #[cfg(all(test, target_os = "linux"))]
        if let Err(error) = &result
            && error.code.as_deref() == Some("native-spawn")
            && error.os_code == Some(libc::ETXTBSY)
        {
            PROBE_BUSY_OBSERVER.with(|observer| {
                if let Some(observer) = observer.borrow_mut().as_mut() {
                    observer(error);
                }
            });
        }
        if held
            .capture_source(NAME, ENGINE_LIMIT)?
            .is_none_or(|(after, image)| after != expected || image != expected_image)
        {
            return Err(infra_core::RuntimeError::conflict(
                "Admitted native probe image changed.",
            ));
        }
        held.check()?;
        #[cfg(unix)]
        let busy = result.as_ref().err().is_some_and(|error| {
            error.code.as_deref() == Some("native-spawn") && error.os_code == Some(libc::ETXTBSY)
        });
        #[cfg(not(unix))]
        let busy = false;
        if !busy {
            return result;
        }
        // Another concurrent fork can briefly retain this image's CLOEXEC
        // writer until its own exec. Retry only a refused exec of the exact
        // same held image; every attempt spends the original deadline.
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(probe_timeout());
        }
        std::thread::sleep(Duration::from_millis(2).min(remaining));
    }
}

#[cfg(all(test, target_os = "linux"))]
type ProbeBusyObserver = Box<dyn FnMut(&infra_core::RuntimeError)>;

#[cfg(all(test, target_os = "linux"))]
thread_local! {
    static PROBE_BUSY_OBSERVER: std::cell::RefCell<Option<ProbeBusyObserver>> =
        const { std::cell::RefCell::new(None) };
}

/// Select a real native artifact explicitly; a test process is never implicitly an engine.
pub fn admit_engine(engine: &Path) -> Result<CreatorEngine> {
    let engine = infra_generator::ownership::resolve_target(engine)?;
    let parent = engine
        .parent()
        .ok_or_else(|| HarnessError::invalid("Native engine has no parent."))?;
    let name = engine
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| HarnessError::invalid("Native engine name is not UTF-8."))?
        .to_string();
    let mut source = PublicationFiles::new(parent)?;
    let (bytes, identity) = source
        .capture_source(&name, ENGINE_LIMIT)?
        .ok_or_else(|| HarnessError::missing("Selected native engine is absent."))?;
    if !native(&bytes) {
        return Err(HarnessError::invalid(
            "Selected engine is not a native executable.",
        ));
    }
    // Probe only a private immutable copy of the captured bytes, never a pathname
    // that can be swapped between admission and execution.
    let probe = tempfile::Builder::new()
        .prefix("creator-engine-admission-")
        .tempdir()?;
    infra_host::util::private_directory(probe.path())?;
    let mut held = PublicationFiles::new(probe.path())?;
    held.write_expected_mode_checked(
        NAME,
        &bytes,
        None,
        None,
        if cfg!(windows) { 0o600 } else { 0o500 },
        ENGINE_LIMIT,
    )?;
    let probe_image = held
        .image_identity(NAME)?
        .ok_or_else(|| HarnessError::conflict("Admitted native probe disappeared."))?;
    let output = run_held_probe(
        &mut held,
        &bytes,
        probe_image,
        &["capabilities".into(), "--json".into()],
        Duration::from_secs(5),
    )?;
    if output.code != 0 || !output.stderr.is_empty() {
        return Err(HarnessError::invalid(
            "Selected native engine capability admission failed.",
        ));
    }
    let capability = crate::strict_json::parse(&output.stdout, 256 * 1024)?;
    if capability["engine"] != "infra-engine"
        || capability["capability_profile"] != "infra-engine-v1"
        || capability["version"].as_str().is_none_or(str::is_empty)
    {
        return Err(HarnessError::invalid(
            "Selected engine has an unsupported native profile.",
        ));
    }
    let declared = capabilities(&capability, true)?;
    let artifact = Artifact::capture(
        parent,
        &name,
        &infra_tooling::artifacts::native_target()?,
        &declared,
    )?;
    if artifact.sha256 != store::sha(&bytes) || artifact.bytes != bytes.len() as u64 {
        return Err(HarnessError::conflict(
            "Selected engine changed during artifact admission.",
        ));
    }
    artifact.verify(parent)?;
    let mut admitted = CreatorEngine {
        bytes: Arc::new(bytes),
        artifact,
        capability,
        source,
        name,
        identity,
        _probe: probe,
    };
    admitted.recheck()?;
    Ok(admitted)
}
impl CreatorEngine {
    fn check(&self) -> Result<()> {
        self.source.check()?;
        Ok(())
    }
    fn recheck(&mut self) -> Result<()> {
        let (_, identity) = self
            .source
            .capture_source(&self.name, ENGINE_LIMIT)?
            .ok_or_else(|| HarnessError::conflict("Selected native engine disappeared."))?;
        if identity != self.identity {
            return Err(HarnessError::conflict(
                "Selected native engine identity changed.",
            ));
        }
        self.check()
    }
    fn descriptor(&self, path: &str) -> Value {
        json!({"schema_version":1,"engine":"infra-engine","capability_profile":"infra-engine-v1",
            "version":self.capability["version"],"target":self.artifact.target,"path":path,
            "sha256":self.artifact.sha256,"bytes":self.artifact.bytes,
            "capabilities":self.capability["capabilities"]})
    }
}
struct Image {
    bytes: Arc<Vec<u8>>,
    mode: u32,
}
struct Tree {
    files: PublicationFiles,
}

fn ignored(path: &str) -> bool {
    path.split('/')
        .any(|part| part == "__pycache__" || part.ends_with(".pyc"))
        || ((path.contains("/memory-bank/local/") || path.contains("/project-brain/local/"))
            && !path.ends_with("/.gitkeep"))
}
fn source_binding(
    source: &mut PublicationFiles,
    binary: &str,
    descriptor: &str,
    declared_path: &str,
    identities: &mut BTreeMap<String, FileImageIdentity>,
) -> Result<()> {
    let descriptor_identity = source.image_identity(descriptor)?;
    let metadata = source.read(descriptor, 1024 * 1024)?;
    if source.image_identity(descriptor)? != descriptor_identity {
        return Err(HarnessError::conflict("Creator source descriptor changed."));
    }
    let image = source.capture_source(binary, ENGINE_LIMIT)?;
    match (metadata, image) {
        (None, None) => Ok(()),
        (Some(raw), Some((bytes, identity))) => {
            let value = crate::strict_json::parse(&raw, 1024 * 1024)?;
            let caps = capabilities(&value, false)?;
            if value["schema_version"] != 1
                || value["engine"] != "infra-engine"
                || value["capability_profile"] != "infra-engine-v1"
                || value["version"].as_str().is_none_or(str::is_empty)
                || value["target"] != infra_tooling::artifacts::native_target()?
                || value["path"] != declared_path
                || value["bytes"].as_u64() != Some(bytes.len() as u64)
                || value["sha256"] != store::sha(&bytes)
                || !native(&bytes)
                || caps.is_empty()
            {
                return Err(HarnessError::conflict(
                    "Creator source native artifact binding is invalid.",
                ));
            }
            // Source release bytes may legitimately differ from the selected
            // Harness build. They are bound and validated, but never executed.
            identities.insert(binary.into(), identity);
            identities.insert(
                descriptor.into(),
                descriptor_identity
                    .ok_or_else(|| HarnessError::conflict("Source descriptor disappeared."))?,
            );
            Ok(())
        }
        _ => Err(HarnessError::conflict(
            "Creator source artifact and descriptor must exist together.",
        )),
    }
}
pub fn copy(source: &Path, destination: &Path, engine: &mut CreatorEngine) -> Result<()> {
    let source = infra_generator::ownership::resolve_target(source)?;
    let destination = infra_generator::ownership::resolve_target(destination)?;
    if source.starts_with(&destination) || destination.starts_with(&source) {
        return Err(HarnessError::forbidden(
            "Creator source and working copy must be disjoint.",
        ));
    }
    let mut rooted = PublicationFiles::new(&source)?;
    engine.recheck()?;
    let mut bindings = vec![(
        format!("memory-bank/bin/{NAME}"),
        "memory-bank/runtime-capabilities.json".to_string(),
        format!("memory-bank/bin/{NAME}"),
    )];
    let mut outputs = BTreeMap::<String, Image>::new();
    let mut source_identities = BTreeMap::new();
    for prefix in [
        ".agents/skills",
        ".claude/skills",
        ".cursor/skills",
        ".codex/skills",
    ] {
        let asset = format!("{prefix}/memory-seed/assets");
        let anchor = format!("{asset}/runtime-contract.json");
        let initial_identity = rooted.image_identity(&anchor)?;
        let present = rooted.read(&anchor, 2 * 1024 * 1024)?.is_some();
        if rooted.image_identity(&anchor)? != initial_identity {
            return Err(HarnessError::conflict(
                "Creator source seed contract changed.",
            ));
        }
        if present {
            source_identities.insert(
                anchor.clone(),
                initial_identity.ok_or_else(|| {
                    HarnessError::conflict("Creator source seed contract has no held identity.")
                })?,
            );
        }
        if prefix == ".agents/skills" && !present {
            return Err(HarnessError::missing(
                "Creator canonical native seed assets are absent.",
            ));
        }
        let binary = format!("{asset}/bin/{NAME}");
        let metadata = format!("{asset}/native-artifacts.json");
        if present {
            bindings.push((binary, metadata, format!("bin/{NAME}")));
        } else if rooted.capture_source(&binary, ENGINE_LIMIT)?.is_some()
            || rooted.read(&metadata, 1024 * 1024)?.is_some()
        {
            return Err(HarnessError::conflict(
                "Creator mirror has an artifact without its seed contract.",
            ));
        }
    }
    for (binary, metadata, declared) in &bindings {
        source_binding(
            &mut rooted,
            binary,
            metadata,
            declared,
            &mut source_identities,
        )?;
        outputs.insert(
            binary.clone(),
            Image {
                bytes: engine.bytes.clone(),
                mode: if cfg!(windows) { 0o644 } else { 0o755 },
            },
        );
        outputs.insert(
            metadata.clone(),
            Image {
                bytes: Arc::new(serde_json::to_vec_pretty(&engine.descriptor(declared))?),
                mode: 0o644,
            },
        );
    }
    let provenance = bindings
        .iter()
        .flat_map(|(binary, metadata, _)| [binary.clone(), metadata.clone()])
        .collect::<BTreeSet<_>>();
    let native_directories = bindings
        .iter()
        .filter_map(|(binary, _, _)| Path::new(binary).parent())
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect::<BTreeSet<_>>();
    let mut total = 0usize;
    let mut count = 0usize;
    for name in ["AGENTS.md", "VERSION", "README.md", CREATOR_CONFIG] {
        let identity = rooted
            .image_identity(name)?
            .ok_or_else(|| HarnessError::missing(format!("Missing Creator source: {name}")))?;
        let bytes = rooted
            .read_checked(
                name,
                if name == CREATOR_CONFIG {
                    CONFIG_LIMIT
                } else {
                    DATA_LIMIT
                },
                |size| {
                    if size > DATA_LIMIT.saturating_sub(total) as u64 {
                        return Err(infra_core::RuntimeError::invalid(
                            "Creator data tree exceeds 128 MiB.",
                        ));
                    }
                    Ok(())
                },
            )?
            .ok_or_else(|| HarnessError::missing("Creator source disappeared."))?;
        if name == CREATOR_CONFIG {
            let configured = crate::strict_json::parse(&bytes, CONFIG_LIMIT)?;
            if !configured.is_object() || configured["framework"] != "creator" {
                return Err(HarnessError::invalid(
                    "Creator source must declare framework creator in its runtime configuration.",
                ));
            }
        }
        total += bytes.len();
        count += 1;
        let mode = rooted
            .read_mode(name)?
            .ok_or_else(|| HarnessError::conflict("Creator mode disappeared."))?;
        source_identities.insert(name.into(), identity);
        outputs.insert(
            name.into(),
            Image {
                bytes: Arc::new(bytes),
                mode,
            },
        );
    }
    let mut trees = Vec::new();
    let mut directories = BTreeSet::from([
        "memory-bank".to_string(),
        "memory-bank/bin".to_string(),
        "tasks".to_string(),
        crate::creator_files::TASK.to_string(),
    ]);
    for prefix in SCOPES {
        let mut tree = PublicationFiles::new(&source.join(prefix))?;
        let (paths, nodes) =
            tree.audit_tree_filtered("", 64, 20000usize.saturating_sub(count), |path| {
                !native_directories.contains(&format!("{prefix}/{path}"))
            })?;
        count = count.saturating_add(nodes).saturating_add(1);
        if count > 20000 {
            return Err(HarnessError::invalid(
                "Creator source exceeds 20000 entries.",
            ));
        }
        directories.insert((*prefix).into());
        for name in paths {
            let path = format!("{prefix}/{name}");
            if ignored(&path) || provenance.contains(&path) {
                continue;
            }
            let identity = tree
                .image_identity(&name)?
                .ok_or_else(|| HarnessError::conflict("Creator source disappeared."))?;
            let bytes = tree
                .read_checked(&name, DATA_LIMIT, |size| {
                    if size > DATA_LIMIT.saturating_sub(total) as u64 {
                        return Err(infra_core::RuntimeError::invalid(
                            "Creator data tree exceeds 128 MiB.",
                        ));
                    }
                    Ok(())
                })?
                .ok_or_else(|| HarnessError::conflict("Creator source disappeared."))?;
            total += bytes.len();
            let mode = tree
                .read_mode(&name)?
                .ok_or_else(|| HarnessError::conflict("Creator source mode disappeared."))?;
            // Keep any identity admitted by the earlier required-anchor probe.
            source_identities.entry(path.clone()).or_insert(identity);
            outputs.insert(
                path,
                Image {
                    bytes: Arc::new(bytes),
                    mode,
                },
            );
        }
        for path in tree.directory_identities()?.keys() {
            if !path.is_empty()
                && !ignored(&format!("{prefix}/{path}"))
                && !native_directories.contains(&format!("{prefix}/{path}"))
            {
                directories.insert(format!("{prefix}/{path}"));
            }
        }
        trees.push(Tree { files: tree });
    }
    for path in outputs.keys() {
        let mut parent = Path::new(path).parent();
        while let Some(path) = parent {
            if path.as_os_str().is_empty() {
                break;
            }
            directories.insert(path.to_string_lossy().replace('\\', "/"));
            parent = path.parent();
        }
    }
    let recheck = |rooted: &mut PublicationFiles, trees: &mut Vec<Tree>| -> Result<()> {
        for (path, identity) in &source_identities {
            if bindings.iter().any(|(binary, _, _)| binary == path) {
                let (_, current) = rooted.capture_source(path, ENGINE_LIMIT)?.ok_or_else(|| {
                    HarnessError::conflict("Creator source artifact disappeared.")
                })?;
                if &current != identity {
                    return Err(HarnessError::conflict("Creator source artifact changed."));
                }
            } else if rooted.image_identity(path)?.as_ref() != Some(identity) {
                return Err(HarnessError::conflict("Creator source image changed."));
            }
        }
        rooted.check()?;
        for tree in trees {
            tree.files.check()?;
        }
        Ok(())
    };
    recheck(&mut rooted, &mut trees)?;
    engine.recheck()?;
    // All source and provenance checks precede the first working-copy write.
    infra_host::util::private_directory(&destination)?;
    let mut target = PublicationFiles::new(&destination)?;
    let mut directories = directories.into_iter().collect::<Vec<_>>();
    directories.sort_by_key(|path| (path.split('/').count(), path.clone()));
    for path in directories {
        target.create_directory(&path)?;
    }
    for (path, image) in outputs {
        target.write_expected_mode_checked(
            &path,
            &image.bytes,
            None,
            None,
            image.mode,
            ENGINE_LIMIT,
        )?;
    }
    recheck(&mut rooted, &mut trees)?;
    engine.recheck()?;
    target.check()?;
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod probe_tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        fs::{File, OpenOptions},
        io,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::{OpenOptionsExt, PermissionsExt},
        },
        rc::Rc,
        sync::{Mutex, MutexGuard},
        time::Instant,
    };

    static INHERITED_WRITER_TESTS: Mutex<()> = Mutex::new(());

    fn exclusive() -> MutexGuard<'static, ()> {
        INHERITED_WRITER_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    struct Probe {
        _root: tempfile::TempDir,
        held: PublicationFiles,
        bytes: Vec<u8>,
        image: FileImageIdentity,
    }
    impl Probe {
        fn new(system: &str, mode: u32) -> Self {
            let source = std::fs::canonicalize(system).unwrap();
            let bytes = infra_core::fs::read_regular(&source).unwrap();
            assert!(
                bytes.starts_with(b"\x7fELF") && bytes.len() <= 1024 * 1024,
                "the private fixture must be a small actual native executable"
            );
            let root = tempfile::Builder::new()
                .prefix("creator-held-probe-")
                .tempdir()
                .unwrap();
            infra_host::util::private_directory(root.path()).unwrap();
            let mut held = PublicationFiles::new(root.path()).unwrap();
            held.write_expected_mode_checked(NAME, &bytes, None, None, mode, ENGINE_LIMIT)
                .unwrap();
            let image = held.image_identity(NAME).unwrap().unwrap();
            Self {
                _root: root,
                held,
                bytes,
                image,
            }
        }
        fn path(&self) -> std::path::PathBuf {
            self.held.root_path().join(NAME)
        }
        fn inherit_writer(&mut self) -> io::Result<InheritedWriter> {
            let path = self.path();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
            let writer = OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
                .open(&path)?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500))?;
            self.image = self
                .held
                .image_identity(NAME)
                .map_err(io::Error::other)?
                .ok_or_else(|| io::Error::other("Private probe disappeared"))?;
            InheritedWriter::fork(writer)
        }
        fn run(
            &mut self,
            timeout: Duration,
        ) -> infra_core::Result<infra_tooling::process::ProcessOutput> {
            run_held_probe(&mut self.held, &self.bytes, self.image, &[], timeout)
        }
    }

    // The child never executes Rust library code after fork. It only proves the
    // inherited descriptor still has CLOEXEC, waits on a private pipe and exits.
    // A direct, unreaped child remains our signal/wait identity throughout.
    struct InheritedWriter {
        pid: Option<libc::pid_t>,
        release: Option<File>,
    }
    impl InheritedWriter {
        fn fork(writer: File) -> io::Result<Self> {
            let pipe = || -> io::Result<(File, File)> {
                let mut descriptors = [-1; 2];
                if unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
                    return Err(io::Error::last_os_error());
                }
                // SAFETY: successful pipe2 returns two newly owned descriptors.
                Ok(unsafe {
                    (
                        File::from_raw_fd(descriptors[0]),
                        File::from_raw_fd(descriptors[1]),
                    )
                })
            };
            let (ready_read, ready_write) = pipe()?;
            let (release_read, release_write) = pipe()?;
            let writer_fd = writer.as_raw_fd();
            let ready_input = ready_read.as_raw_fd();
            let ready_output = ready_write.as_raw_fd();
            let release_input = release_read.as_raw_fd();
            let release_output = release_write.as_raw_fd();
            // SAFETY: the child below uses only async-signal-safe libc calls;
            // no allocation, locks, destructors or Rust test assertions run there.
            let pid = unsafe { libc::fork() };
            if pid < 0 {
                return Err(io::Error::last_os_error());
            }
            if pid == 0 {
                unsafe {
                    libc::close(ready_input);
                    libc::close(release_output);
                    let flags = libc::fcntl(writer_fd, libc::F_GETFD);
                    let ready = if flags >= 0 && flags & libc::FD_CLOEXEC != 0 {
                        1u8
                    } else {
                        0u8
                    };
                    if libc::write(ready_output, (&ready as *const u8).cast(), 1) != 1 {
                        libc::_exit(2);
                    }
                    let mut byte = 0u8;
                    loop {
                        let count = libc::read(release_input, (&mut byte as *mut u8).cast(), 1);
                        if count < 0 && *libc::__errno_location() == libc::EINTR {
                            continue;
                        }
                        break;
                    }
                    libc::close(writer_fd);
                    libc::close(ready_output);
                    libc::close(release_input);
                    libc::_exit(if ready == 1 { 0 } else { 3 });
                }
            }
            let mut child = Self {
                pid: Some(pid),
                release: Some(release_write),
            };
            drop(writer);
            drop(ready_write);
            drop(release_read);
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut poll = libc::pollfd {
                fd: ready_read.as_raw_fd(),
                events: libc::POLLIN | libc::POLLHUP,
                revents: 0,
            };
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    let _ = child.release_and_wait();
                    return Err(io::Error::other("Inherited writer did not become ready"));
                }
                let polled =
                    unsafe { libc::poll(&mut poll, 1, left.as_millis().clamp(1, 20) as i32) };
                if polled < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                if polled < 0 {
                    return Err(io::Error::last_os_error());
                }
                if polled > 0 {
                    let mut ready = 0u8;
                    let read = unsafe {
                        libc::read(ready_read.as_raw_fd(), (&mut ready as *mut u8).cast(), 1)
                    };
                    if read == 1 && ready == 1 {
                        return Ok(child);
                    }
                    return Err(io::Error::other("Inherited writer did not retain CLOEXEC"));
                }
            }
        }
        fn release_and_wait(&mut self) -> io::Result<()> {
            self.release.take();
            let Some(pid) = self.pid else {
                return Ok(());
            };
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                let mut status = 0;
                let waited = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
                if waited == pid {
                    self.pid = None;
                    if status == 0 {
                        return Ok(());
                    }
                    return Err(io::Error::other("Inherited writer child failed"));
                }
                if waited < 0 {
                    if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(io::Error::last_os_error());
                }
                if Instant::now() >= deadline {
                    // This exact child is still unreaped; no recycled PID is
                    // eligible for this targeted fallback signal.
                    if unsafe { libc::kill(pid, libc::SIGKILL) } != 0 {
                        return Err(io::Error::last_os_error());
                    }
                    let until = Instant::now() + Duration::from_secs(2);
                    loop {
                        let mut status = 0;
                        let waited = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
                        if waited == pid {
                            self.pid = None;
                            return Ok(());
                        }
                        if waited < 0
                            && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted
                        {
                            return Err(io::Error::last_os_error());
                        }
                        if Instant::now() >= until {
                            return Err(io::Error::other(
                                "Owned inherited writer cleanup unconfirmed",
                            ));
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
    impl Drop for InheritedWriter {
        fn drop(&mut self) {
            let _ = self.release_and_wait();
        }
    }

    struct Observer(Option<ProbeBusyObserver>);
    impl Observer {
        fn install(callback: impl FnMut(&infra_core::RuntimeError) + 'static) -> Self {
            Self(PROBE_BUSY_OBSERVER.with(|observer| observer.replace(Some(Box::new(callback)))))
        }
    }
    impl Drop for Observer {
        fn drop(&mut self) {
            PROBE_BUSY_OBSERVER.with(|observer| {
                observer.replace(self.0.take());
            });
        }
    }

    #[test]
    fn inherited_cloexec_writer_releases_then_same_probe_succeeds() {
        let _serial = exclusive();
        let mut probe = Probe::new("/usr/bin/true", 0o500);
        let holder = Rc::new(RefCell::new(probe.inherit_writer().unwrap()));
        let observed = Rc::new(Cell::new(0usize));
        let release_error = Rc::new(RefCell::new(None));
        let observed_errno = Rc::new(Cell::new(None));
        let observer = Observer::install({
            let holder = holder.clone();
            let observed = observed.clone();
            let release_error = release_error.clone();
            let observed_errno = observed_errno.clone();
            move |error| {
                observed_errno.set(error.os_code);
                observed.set(observed.get() + 1);
                if let Err(error) = holder.borrow_mut().release_and_wait() {
                    *release_error.borrow_mut() = Some(error.to_string());
                }
            }
        });
        let result = probe.run(Duration::from_secs(2));
        drop(observer);
        let cleanup = holder.borrow_mut().release_and_wait();
        assert!(cleanup.is_ok(), "owned child cleanup: {cleanup:?}");
        assert!(
            release_error.borrow().is_none(),
            "release failed: {:?}",
            release_error.borrow()
        );
        assert_eq!(
            observed.get(),
            1,
            "the first attempt must really hit ETXTBSY"
        );
        assert_eq!(observed_errno.get(), Some(libc::ETXTBSY));
        assert_eq!(result.unwrap().code, 0);
    }

    #[test]
    fn persistent_inherited_writer_uses_only_the_original_deadline() {
        let _serial = exclusive();
        let mut probe = Probe::new("/usr/bin/true", 0o500);
        let mut holder = probe.inherit_writer().unwrap();
        let started = Instant::now();
        let result = probe.run(Duration::from_millis(80));
        let elapsed = started.elapsed();
        let cleanup = holder.release_and_wait();
        assert!(cleanup.is_ok(), "owned child cleanup: {cleanup:?}");
        let error = result.unwrap_err();
        assert_eq!(error.code.as_deref(), Some("native-timeout"));
        assert!(
            elapsed < Duration::from_secs(1),
            "probe extended its deadline: {elapsed:?}"
        );
    }

    #[test]
    fn busy_probe_rejects_replaced_changed_or_repermissioned_images() {
        let _serial = exclusive();
        for mutation in ["replace", "bytes", "mode"] {
            let mut probe = Probe::new("/usr/bin/true", 0o500);
            let holder = Rc::new(RefCell::new(probe.inherit_writer().unwrap()));
            let path = probe.path();
            let expected = probe.bytes.clone();
            let mutation_error = Rc::new(RefCell::new(None));
            let observed = Rc::new(Cell::new(0usize));
            let observer = Observer::install({
                let holder = holder.clone();
                let mutation_error = mutation_error.clone();
                let observed = observed.clone();
                move |_| {
                    observed.set(observed.get() + 1);
                    let result = (|| -> io::Result<()> {
                        holder.borrow_mut().release_and_wait()?;
                        match mutation {
                            "replace" => {
                                std::fs::rename(&path, path.with_extension("retained"))?;
                                std::fs::write(&path, &expected)?;
                                std::fs::set_permissions(
                                    &path,
                                    std::fs::Permissions::from_mode(0o500),
                                )?;
                            }
                            "bytes" => {
                                std::fs::set_permissions(
                                    &path,
                                    std::fs::Permissions::from_mode(0o700),
                                )?;
                                let mut changed = expected.clone();
                                changed[0] ^= 1;
                                std::fs::write(&path, changed)?;
                                std::fs::set_permissions(
                                    &path,
                                    std::fs::Permissions::from_mode(0o500),
                                )?;
                            }
                            "mode" => {
                                std::fs::set_permissions(
                                    &path,
                                    std::fs::Permissions::from_mode(0o700),
                                )?;
                            }
                            _ => unreachable!(),
                        }
                        Ok(())
                    })();
                    if let Err(error) = result {
                        *mutation_error.borrow_mut() = Some(error.to_string());
                    }
                }
            });
            let result = probe.run(Duration::from_secs(2));
            drop(observer);
            let cleanup = holder.borrow_mut().release_and_wait();
            assert!(
                cleanup.is_ok(),
                "{mutation} owned child cleanup: {cleanup:?}"
            );
            assert!(
                mutation_error.borrow().is_none(),
                "{mutation}: {:?}",
                mutation_error.borrow()
            );
            assert_eq!(
                observed.get(),
                1,
                "{mutation} must follow an actual busy attempt"
            );
            assert_eq!(
                result.unwrap_err().kind,
                infra_core::ErrorKind::Conflict,
                "{mutation}"
            );
        }
    }

    #[test]
    fn permission_error_preserves_raw_errno_without_busy_retry() {
        let _serial = exclusive();
        let mut probe = Probe::new("/usr/bin/true", 0o400);
        let result = probe.run(Duration::from_secs(2));
        let error = result.unwrap_err();
        assert_eq!(error.code.as_deref(), Some("native-spawn"));
        assert_eq!(error.os_code, Some(libc::EACCES));
    }

    #[test]
    fn nonzero_native_exit_is_not_a_busy_spawn_failure() {
        let _serial = exclusive();
        let mut probe = Probe::new("/usr/bin/false", 0o500);
        let result = probe.run(Duration::from_secs(2)).unwrap();
        assert_eq!(result.code, 1);
        assert!(result.stdout.is_empty());
        assert!(result.stderr.is_empty());
    }
}
