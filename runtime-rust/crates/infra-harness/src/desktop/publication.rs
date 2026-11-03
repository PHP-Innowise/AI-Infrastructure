//! Closed desktop ownership roster and held input images.
//! No record member grants deletion authority outside these platform paths.
use super::{HarnessError, Result, config_directory, home};
use infra_core::bank::publication::{FileImageIdentity, PublicationFiles, commit::ExpectedImage};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

const ICON_LIMIT: usize = 8 * 1024 * 1024;
const RECORD_LIMIT: usize = 65536;
const OUTPUT_LIMIT: usize = 16 * 1024 * 1024;
const PNG_SIZES: [u32; 10] = [16, 22, 24, 32, 48, 64, 128, 256, 512, 1024];
#[cfg(target_os = "macos")]
const BUNDLE_ID: &str = "local.ai-infrastructure.accelerator";

#[cfg(target_os = "linux")]
pub(super) fn desktop_exec(executable: &Path) -> Result<String> {
    let value = executable
        .to_str()
        .ok_or_else(|| HarnessError::invalid("Desktop executable path is not Unicode."))?;
    if value.contains('=')
        || value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\t' | '\r'))
    {
        return Err(HarnessError::invalid(
            "Desktop executable path contains an unsupported character.",
        ));
    }
    // Argument quoting precedes string-value escaping: two distinct layers.
    let reserved = " \t\n\r\"'\\><~|&;$*?#()`";
    let argument = if value.chars().any(|character| reserved.contains(character)) {
        let mut quoted = String::from("\"");
        for character in value.chars() {
            if matches!(character, '"' | '`' | '$' | '\\') {
                quoted.push('\\');
            }
            quoted.push(character);
        }
        quoted.push('"');
        quoted
    } else {
        value.to_string()
    };
    let command = format!("{} app open", argument.replace('%', "%%"));
    Ok(command
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
        .replace('\r', "\\r"))
}

struct SourceImage {
    files: PublicationFiles,
    name: String,
    bytes: Vec<u8>,
    identity: FileImageIdentity,
}
impl SourceImage {
    fn read(path: &Path, limit: usize) -> Result<Self> {
        let path = absolute(path)?;
        let parent = path
            .parent()
            .ok_or_else(|| HarnessError::invalid("Input has no parent."))?;
        let name = path
            .file_name()
            .and_then(|part| part.to_str())
            .ok_or_else(|| HarnessError::invalid("Input name is unavailable."))?
            .to_string();
        let mut files = PublicationFiles::new(parent)?;
        let (bytes, identity) = files
            .capture_source(&name, limit)?
            .ok_or_else(|| HarnessError::missing("A required desktop input is missing."))?;
        files.check()?;
        Ok(Self {
            files,
            name,
            bytes,
            identity,
        })
    }
    fn check(&mut self, limit: usize) -> Result<()> {
        let current = self.files.capture_source(&self.name, limit)?;
        if current
            .as_ref()
            .map(|(bytes, identity)| (bytes.as_slice(), *identity))
            != Some((self.bytes.as_slice(), self.identity))
        {
            return Err(HarnessError::conflict(
                "A desktop input changed during admission.",
            ));
        }
        self.files.check()?;
        Ok(())
    }
}
pub(super) struct Inputs {
    images: BTreeMap<u32, SourceImage>,
    svg: SourceImage,
    engine_image: SourceImage,
    pub(super) executable: PathBuf,
    #[cfg(windows)]
    worker: PathBuf,
    #[cfg(windows)]
    worker_image: SourceImage,
}
impl Inputs {
    pub(super) fn admit(source: &Path, engine: &Path) -> Result<Self> {
        // Canonical install preflights every shipped icon on every platform.
        let mut images = BTreeMap::new();
        let svg = SourceImage::read(
            &source.join("harness/web/icons/ai-accelerator.svg"),
            ICON_LIMIT,
        )?;
        for size in PNG_SIZES {
            images.insert(
                size,
                SourceImage::read(
                    &source.join(format!("harness/web/icons/ai-accelerator-{size}.png")),
                    ICON_LIMIT,
                )?,
            );
        }
        let executable = std::fs::canonicalize(engine)?;
        let engine_image = SourceImage::read(&executable, 512 * 1024 * 1024)?;
        #[cfg(windows)]
        let worker = std::env::current_exe()?;
        #[cfg(windows)]
        let worker_image = SourceImage::read(&worker, 512 * 1024 * 1024)?;
        let mut result = Self {
            images,
            svg,
            engine_image,
            executable,
            #[cfg(windows)]
            worker,
            #[cfg(windows)]
            worker_image,
        };
        result.check()?;
        Ok(result)
    }
    pub(super) fn icon(&self, size: u32) -> Result<&[u8]> {
        self.images
            .get(&size)
            .map(|image| image.bytes.as_slice())
            .ok_or_else(|| HarnessError::missing("A required desktop icon was not admitted."))
    }
    #[cfg(target_os = "linux")]
    pub(super) fn svg(&self) -> &[u8] {
        &self.svg.bytes
    }
    pub(super) fn check(&mut self) -> Result<()> {
        self.svg.check(ICON_LIMIT)?;
        for image in self.images.values_mut() {
            image.check(ICON_LIMIT)?;
        }
        self.engine_image.check(512 * 1024 * 1024)?;
        #[cfg(windows)]
        self.worker_image.check(512 * 1024 * 1024)?;
        Ok(())
    }
}

fn absolute(path: &Path) -> Result<PathBuf> {
    let path = infra_core::fs::absolute(path)?;
    if path
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(HarnessError::invalid(
            "Desktop path contains a parent traversal.",
        ));
    }
    Ok(path)
}
fn relative(path: &Path) -> Result<String> {
    let text = path
        .to_str()
        .ok_or_else(|| HarnessError::invalid("Desktop path is not Unicode."))?;
    #[cfg(windows)]
    let text = text.replace('\\', "/");
    #[cfg(not(windows))]
    let text = text.to_string();
    if text.is_empty()
        || text
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || text.contains(['\\', '\0'])
    {
        return Err(HarnessError::invalid("Invalid desktop publication path."));
    }
    Ok(text)
}
enum TargetName {
    Relative(String),
    /// No path-based operation may publish/delete through this witness. The
    /// retained ancestor and the first absent native component are rechecked.
    MissingNativeParent(PathBuf),
}
struct Target {
    absolute: PathBuf,
    files: PublicationFiles,
    name: TargetName,
    bytes: Option<Vec<u8>>,
    mode: Option<u32>,
    identity: Option<FileImageIdentity>,
}
impl Target {
    fn admit(path: &Path, limit: usize) -> Result<Self> {
        let path = absolute(path)?;
        let parent = path
            .parent()
            .ok_or_else(|| HarnessError::invalid("Desktop target has no parent."))?
            .to_path_buf();
        let mut root = parent.clone();
        loop {
            match std::fs::symlink_metadata(&root) {
                Ok(_) => break,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if !root.pop() {
                        return Err(HarnessError::missing(
                            "Desktop target parent is unavailable.",
                        ));
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        let suffix = path
            .strip_prefix(&root)
            .map_err(|_| HarnessError::forbidden("Desktop target escaped its parent."))?;
        let mut files = PublicationFiles::new(&root)?;
        let name = match relative(suffix) {
            Ok(name) => name,
            Err(_) if root != parent => {
                // Native roots may contain a literal Unix backslash. A missing
                // root is an absence witness, not a portable publication key.
                let Some(std::path::Component::Normal(first)) = suffix.components().next() else {
                    return Err(HarnessError::invalid("Invalid native desktop parent."));
                };
                let missing = root.join(first);
                match std::fs::symlink_metadata(&missing) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                    Ok(_) => {
                        return Err(HarnessError::conflict(
                            "Desktop parent changed during admission.",
                        ));
                    }
                }
                files.check()?;
                return Ok(Self {
                    absolute: path,
                    files,
                    name: TargetName::MissingNativeParent(missing),
                    bytes: None,
                    mode: None,
                    identity: None,
                });
            }
            Err(error) => return Err(error),
        };
        let identity = files.image_identity(&name)?;
        let bytes = files.read(&name, limit)?;
        let mode = files.read_mode(&name)?;
        if files.image_identity(&name)? != identity
            || bytes.is_some() != identity.is_some()
            || bytes.is_some() != mode.is_some()
        {
            return Err(HarnessError::conflict(
                "Desktop target changed during admission.",
            ));
        }
        files.check()?;
        Ok(Self {
            absolute: path,
            files,
            name: TargetName::Relative(name),
            bytes,
            mode,
            identity,
        })
    }
    fn assert_absent(&mut self) -> Result<()> {
        self.files.check()?;
        match &self.name {
            TargetName::Relative(name) => {
                if self.files.image_identity(name)?.is_some() {
                    return Err(HarnessError::conflict(
                        "An absent desktop target appeared during uninstall.",
                    ));
                }
            }
            TargetName::MissingNativeParent(missing) => match std::fs::symlink_metadata(missing) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
                Ok(_) => {
                    return Err(HarnessError::conflict(
                        "An absent desktop parent appeared during uninstall.",
                    ));
                }
            },
        }
        self.files.check()?;
        Ok(())
    }
    fn refuse_pending_removal(&mut self) -> Result<()> {
        let name = match &self.name {
            TargetName::Relative(name) => name.clone(),
            TargetName::MissingNativeParent(_) => return self.assert_absent(),
        };
        let (parent, leaf) = name.rsplit_once('/').unwrap_or(("", name.as_str()));
        if self.files.archive_directory_exists(parent)? {
            let prefix = format!(".{leaf}.");
            for name in self.files.archive_entries(parent, 1_000_000)? {
                if name.starts_with(&prefix) && name.ends_with(".removed") {
                    return Err(HarnessError::conflict(
                        "A prior desktop removal is incomplete; its retained image and installation record require recovery.",
                    ));
                }
            }
        }
        Ok(())
    }
    fn remove(&mut self, limit: usize) -> Result<bool> {
        if self.bytes.is_none() {
            self.assert_absent()?;
            return Ok(false);
        }
        let name = match &self.name {
            TargetName::Relative(name) => name,
            TargetName::MissingNativeParent(_) => {
                return Err(HarnessError::conflict(
                    "An absent desktop parent has an inconsistent image.",
                ));
            }
        };
        let expected = ExpectedImage {
            bytes: self.bytes.as_deref(),
            mode: self.mode,
            identity: self.identity,
        };
        self.files
            .remove_reviewed_image(name, expected, limit, |_| Ok(()))?;
        self.bytes = None;
        self.mode = None;
        self.identity = None;
        Ok(true)
    }
}

struct PublicationTarget {
    root: usize,
    name: String,
    bytes: Option<Vec<u8>>,
    mode: Option<u32>,
    identity: Option<FileImageIdentity>,
}
pub(super) struct Publisher {
    roots: Vec<PublicationFiles>,
    targets: BTreeMap<PathBuf, PublicationTarget>,
}
impl Publisher {
    pub(super) fn admit(
        paths: impl IntoIterator<Item = PathBuf>,
        mut roots: Vec<PublicationFiles>,
    ) -> Result<Self> {
        for root in &roots {
            root.check()?;
        }
        let mut targets = BTreeMap::new();
        for path in paths {
            let path = absolute(&path)?;
            let index = roots
                .iter()
                .enumerate()
                .filter(|(_, root)| {
                    path.as_path() != root.root_path() && path.starts_with(root.root_path())
                })
                .max_by_key(|(_, root)| root.root_path().components().count())
                .map(|(index, _)| index)
                .ok_or_else(|| {
                    HarnessError::forbidden("Desktop target has no retained publication root.")
                })?;
            let root = &mut roots[index];
            let name = relative(
                path.strip_prefix(root.root_path())
                    .map_err(|_| HarnessError::forbidden("Desktop target escaped its root."))?,
            )?;
            let identity = root.image_identity(&name)?;
            let bytes = root.read(&name, OUTPUT_LIMIT)?;
            let mode = root.read_mode(&name)?;
            if root.image_identity(&name)? != identity
                || bytes.is_some() != mode.is_some()
                || bytes.is_some() != identity.is_some()
            {
                return Err(HarnessError::conflict(
                    "Desktop target changed during admission.",
                ));
            }
            if targets
                .insert(
                    path,
                    PublicationTarget {
                        root: index,
                        name,
                        bytes,
                        mode,
                        identity,
                    },
                )
                .is_some()
            {
                return Err(HarnessError::invalid(
                    "Duplicate desktop publication target.",
                ));
            }
        }
        let result = Self { roots, targets };
        result.check()?;
        Ok(result)
    }
    pub(super) fn write(&mut self, path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
        self.check()?;
        let target = self
            .targets
            .get_mut(&absolute(path)?)
            .ok_or_else(|| HarnessError::forbidden("Desktop target was not admitted."))?;
        let expected = ExpectedImage {
            bytes: target.bytes.as_deref(),
            mode: target.mode,
            identity: target.identity,
        };
        let published = self.roots[target.root].write_reviewed_image(
            &target.name,
            bytes,
            expected,
            mode,
            OUTPUT_LIMIT,
            |_| Ok(()),
        )?;
        target.bytes = Some(bytes.to_vec());
        target.mode = Some(published.mode);
        target.identity = Some(published.identity);
        self.check()
    }
    pub(super) fn check(&self) -> Result<()> {
        for root in &self.roots {
            root.check()?;
        }
        Ok(())
    }
}
fn public_output_root() -> Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        absolute(
            &std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or(home()?.join(".local/share")),
        )
    }
    #[cfg(target_os = "macos")]
    {
        absolute(&home()?.join("Applications"))
    }
    #[cfg(windows)]
    {
        absolute(
            &std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .unwrap_or(home()?.join("AppData/Roaming")),
        )
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        Err(HarnessError::unavailable(
            "Desktop integration is unavailable on this platform.",
        ))
    }
}
pub(super) fn install_roots(config: &Path) -> Result<Vec<PublicationFiles>> {
    use infra_core::bank::publication::private::RootProtection;
    let config = absolute(config)?;
    let public = public_output_root()?;
    // Settle the selected config's authorized protection before capturing a
    // public root which may be its descendant or ancestor.
    let mut roots = vec![PublicationFiles::open_directory_root(
        &config,
        RootProtection::SelectedRoot,
    )?];
    if public != config {
        roots.push(PublicationFiles::open_directory_root(
            &public,
            RootProtection::CreatedDirectories,
        )?);
    }
    for root in &roots {
        root.check()?;
    }
    Ok(roots)
}
pub(super) fn install_paths(config: &Path) -> Result<Vec<PathBuf>> {
    let layout = Layout::current()?;
    let mut paths = layout.paths;
    #[cfg(target_os = "macos")]
    {
        paths.clear();
        paths.extend(
            [
                "Contents/MacOS/accelerator",
                "Contents/Info.plist",
                "Contents/Resources/ai-accelerator.icns",
            ]
            .map(|name| layout.bundle.join(name)),
        );
    }
    paths.push(config.join("app.json"));
    Ok(paths)
}

pub(super) fn installation_record(source: &Path, executable: &Path) -> Result<Vec<u8>> {
    let value = serde_json::json!({"clone":source,"executable":executable,
        "path":std::env::var("PATH").unwrap_or_default(),"installed":Layout::current()?.paths});
    let bytes = format!("{}\n", infra_core::domain::json::sorted_pretty(&value)?).into_bytes();
    if bytes.len() > RECORD_LIMIT {
        return Err(HarnessError::invalid(
            "Desktop installation record exceeds its read boundary.",
        ));
    }
    Ok(bytes)
}

/// The roster derives only from the platform layout selector, never from app.json.
struct Layout {
    record: PathBuf,
    paths: Vec<PathBuf>,
    #[cfg(target_os = "macos")]
    bundle: PathBuf,
}
impl Layout {
    fn current() -> Result<Self> {
        let config = absolute(&config_directory()?)?;
        #[cfg(target_os = "linux")]
        let paths = {
            let data = absolute(
                &std::env::var_os("XDG_DATA_HOME")
                    .map(PathBuf::from)
                    .unwrap_or(home()?.join(".local/share")),
            )?;
            let mut paths = vec![
                data.join("applications/ai-accelerator.desktop"),
                data.join("icons/hicolor/scalable/apps/ai-accelerator.svg"),
            ];
            paths.extend([16, 22, 24, 32, 48, 64, 128, 256, 512].map(|size| {
                data.join(format!(
                    "icons/hicolor/{size}x{size}/apps/ai-accelerator.png"
                ))
            }));
            paths
        };
        #[cfg(windows)]
        let paths = {
            let appdata = absolute(
                &std::env::var_os("APPDATA")
                    .map(PathBuf::from)
                    .unwrap_or(home()?.join("AppData/Roaming")),
            )?;
            vec![
                appdata.join("Microsoft/Windows/Start Menu/Programs/AI Accelerator.lnk"),
                config.join("ai-accelerator.ico"),
            ]
        };
        #[cfg(target_os = "macos")]
        let bundle = absolute(&home()?.join("Applications/AI Accelerator.app"))?;
        #[cfg(target_os = "macos")]
        let paths = vec![bundle.clone()];
        #[cfg(any(target_os = "linux", target_os = "macos", windows))]
        {
            Ok(Self {
                record: config.join("app.json"),
                paths,
                #[cfg(target_os = "macos")]
                bundle,
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            Err(HarnessError::unavailable(
                "Desktop integration is unavailable on this platform.",
            ))
        }
    }
    fn validate_record(&self, record: &Value) -> Result<()> {
        let map = record
            .as_object()
            .ok_or_else(|| HarnessError::invalid("Invalid installation record."))?;
        let native = map.contains_key("installed");
        let allowed: &[&str] = if native {
            &["clone", "path", "executable", "installed", "installed_at"]
        } else {
            &["clone", "path", "installed_at"]
        };
        if map.keys().any(|key| !allowed.contains(&key.as_str()))
            || !record["clone"]
                .as_str()
                .is_some_and(|text| !text.contains('\0') && Path::new(text).is_absolute())
            || !record["path"].is_string()
            || (!native && !record["installed_at"].is_string())
            || (map.contains_key("installed_at") && !record["installed_at"].is_string())
        {
            return Err(HarnessError::invalid("Invalid installation record."));
        }
        if native {
            if !record["executable"]
                .as_str()
                .is_some_and(|text| !text.contains('\0') && Path::new(text).is_absolute())
            {
                return Err(HarnessError::invalid("Invalid native installation record."));
            }
            let listed = record["installed"]
                .as_array()
                .ok_or_else(|| HarnessError::invalid("Invalid installation target roster."))?;
            let expected = self.paths.iter().cloned().collect::<BTreeSet<_>>();
            let mut seen = BTreeSet::new();
            for value in listed {
                let text = value
                    .as_str()
                    .ok_or_else(|| HarnessError::invalid("Invalid installation target."))?;
                let path = PathBuf::from(text);
                if text.contains('\0')
                    || !path.is_absolute()
                    || absolute(&path)? != path
                    || !expected.contains(&path)
                    || !seen.insert(path)
                {
                    return Err(HarnessError::forbidden(
                        "Installation record contains an unowned desktop target.",
                    ));
                }
            }
        }
        Ok(())
    }
}

pub(super) struct Uninstall {
    record: Target,
    targets: Vec<Target>,
    #[cfg(target_os = "macos")]
    bundle: Bundle,
}
impl Uninstall {
    pub(super) fn prepare() -> Result<Option<Self>> {
        let layout = Layout::current()?;
        let mut record = Target::admit(&layout.record, RECORD_LIMIT)?;
        record.refuse_pending_removal()?;
        if let Some(raw) = record.bytes.as_deref() {
            let value = crate::strict_json::parse(raw, RECORD_LIMIT)?;
            layout.validate_record(&value)?;
        }
        #[cfg(target_os = "macos")]
        let (bundle, mut targets) = Bundle::admit(&layout.bundle)?;
        #[cfg(not(target_os = "macos"))]
        let mut targets = layout
            .paths
            .iter()
            .map(|path| Target::admit(path, OUTPUT_LIMIT))
            .collect::<Result<Vec<_>>>()?;
        for target in &mut targets {
            target.refuse_pending_removal()?;
        }
        // Admit every target before deleting any of them. Missing owned names
        // are legitimate after an interrupted earlier uninstall.
        if record.bytes.is_none() {
            let orphaned = targets.iter().any(|target| target.bytes.is_some());
            #[cfg(target_os = "macos")]
            let orphaned = orphaned || bundle.identities.contains_key(&bundle.prefix);
            if orphaned {
                return Err(HarnessError::missing(
                    "Desktop targets remain without an installation record; preserve them for ownership review.",
                ));
            }
            return Ok(None);
        }
        Ok(Some(Self {
            record,
            targets,
            #[cfg(target_os = "macos")]
            bundle,
        }))
    }
    pub(super) fn finish<F>(mut self, external: F) -> Result<Vec<PathBuf>>
    where
        F: FnOnce() -> Result<()>,
    {
        let mut removed = Vec::new();
        for target in &mut self.targets {
            if target.remove(OUTPUT_LIMIT)? {
                removed.push(target.absolute.clone());
            }
        }
        #[cfg(target_os = "macos")]
        {
            let existed = self.bundle.identities.contains_key(&self.bundle.prefix);
            self.bundle.remove_empty()?;
            removed = if existed {
                vec![self.bundle.absolute.clone()]
            } else {
                Vec::new()
            };
        }
        #[cfg(not(target_os = "macos"))]
        for target in &mut self.targets {
            target.assert_absent()?;
        }
        external()?;
        self.record.remove(RECORD_LIMIT)?;
        Ok(removed)
    }
}

#[cfg(target_os = "macos")]
struct Bundle {
    absolute: PathBuf,
    files: PublicationFiles,
    prefix: String,
    identities: BTreeMap<String, (u64, u64, u32)>,
}
#[cfg(target_os = "macos")]
impl Bundle {
    fn admit(bundle: &Path) -> Result<(Self, Vec<Target>)> {
        let parent = bundle
            .parent()
            .ok_or_else(|| HarnessError::invalid("Bundle has no parent."))?;
        // Admit from the closest existing ordinary ancestor without mkdir.
        let mut anchor = parent.to_path_buf();
        while !anchor.exists() {
            if !anchor.pop() {
                return Err(HarnessError::missing("Bundle parent unavailable."));
            }
        }
        let prefix = relative(
            bundle
                .strip_prefix(&anchor)
                .map_err(|_| HarnessError::forbidden("Bundle escaped its parent."))?,
        )?;
        let mut files = PublicationFiles::new(&anchor)?;
        let mut targets = Vec::new();
        if files.archive_directory_exists(&prefix)? {
            for (directory, allowed) in [
                ("", vec!["Contents"]),
                ("Contents", vec!["MacOS", "Resources", "Info.plist"]),
                ("Contents/MacOS", vec!["accelerator", "ai-accelerator"]),
                (
                    "Contents/Resources",
                    vec!["ai-accelerator.icns", "AppIcon.icns"],
                ),
            ] {
                let path = if directory.is_empty() {
                    prefix.clone()
                } else {
                    format!("{prefix}/{directory}")
                };
                if files.archive_directory_exists(&path)? {
                    for name in files.archive_entries(&path, 100)? {
                        if !allowed.contains(&name.as_str()) {
                            return Err(HarnessError::forbidden(
                                "The application bundle contains unrelated files; it was preserved.",
                            ));
                        }
                    }
                }
            }
            // Info.plist is deliberately deleted last, preserving bundle
            // ownership evidence if an earlier removal needs a retry.
            for name in [
                "Contents/MacOS/accelerator",
                "Contents/MacOS/ai-accelerator",
                "Contents/Resources/ai-accelerator.icns",
                "Contents/Resources/AppIcon.icns",
                "Contents/Info.plist",
            ] {
                targets.push(Target::admit(&bundle.join(name), OUTPUT_LIMIT)?);
            }
            let plist = targets.last().and_then(|target| target.bytes.as_deref());
            if let Some(plist) = plist {
                validate_bundle_identifier(plist)?;
            } else if targets.iter().any(|target| target.bytes.is_some()) {
                return Err(HarnessError::forbidden(
                    "Application bundle ownership is unavailable; it was preserved.",
                ));
            }
        }
        let identities = files.directory_identities()?;
        Ok((
            Self {
                absolute: bundle.to_path_buf(),
                files,
                prefix,
                identities,
            },
            targets,
        ))
    }
    fn remove_empty(&mut self) -> Result<()> {
        for suffix in ["/Contents/MacOS", "/Contents/Resources", "/Contents", ""] {
            let name = format!("{}{suffix}", self.prefix);
            if let Some(identity) = self.identities.get(&name)
                && !self
                    .files
                    .remove_empty_directory_expected(&name, *identity)?
            {
                return Err(HarnessError::conflict(
                    "Application bundle changed during uninstall.",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn validate_bundle_identifier(raw: &[u8]) -> Result<()> {
    use quick_xml::{Reader, events::Event, name::QName};
    let text = std::str::from_utf8(raw)
        .map_err(|_| HarnessError::invalid("Invalid application bundle metadata."))?;
    if raw.len() > RECORD_LIMIT {
        return Err(HarnessError::invalid(
            "Application bundle metadata exceeds its boundary.",
        ));
    }
    let mut reader = Reader::from_str(text);
    let mut depth = 0usize;
    let mut nodes = 0usize;
    let mut identifier = None;
    let mut root_seen = false;
    let mut dictionary_seen = false;
    let mut expecting_identifier = false;
    loop {
        nodes += 1;
        if nodes > 4096 || depth > 128 {
            return Err(HarnessError::invalid(
                "Invalid application bundle metadata.",
            ));
        }
        let event = reader
            .read_event()
            .map_err(|_| HarnessError::invalid("Invalid application bundle metadata."))?;
        match event {
            Event::Start(start) if depth == 2 && start.name() == QName(b"key") => {
                let value = reader
                    .read_text(QName(b"key"))
                    .map_err(|_| HarnessError::invalid("Invalid application bundle metadata."))?;
                let key = quick_xml::escape::unescape(&value)
                    .map_err(|_| HarnessError::invalid("Invalid application bundle metadata."))?;
                if expecting_identifier {
                    return Err(HarnessError::invalid("Missing bundle identifier value."));
                }
                expecting_identifier = key == "CFBundleIdentifier";
            }
            Event::Start(start) if depth == 2 && expecting_identifier => {
                if start.name() != QName(b"string") || identifier.is_some() {
                    return Err(HarnessError::invalid("Invalid bundle identifier value."));
                }
                let value = reader
                    .read_text(QName(b"string"))
                    .map_err(|_| HarnessError::invalid("Invalid application bundle metadata."))?;
                identifier = Some(
                    quick_xml::escape::unescape(&value)
                        .map_err(|_| HarnessError::invalid("Invalid application bundle metadata."))?
                        .into_owned(),
                );
                expecting_identifier = false;
            }
            Event::Start(start) => {
                if (depth == 0 && (root_seen || start.name() != QName(b"plist")))
                    || (depth == 1 && (dictionary_seen || start.name() != QName(b"dict")))
                {
                    return Err(HarnessError::invalid(
                        "Invalid application bundle metadata root.",
                    ));
                }
                if depth == 0 {
                    root_seen = true;
                }
                if depth == 1 {
                    dictionary_seen = true;
                }
                depth += 1;
            }
            Event::Text(text) if depth == 0 => {
                if !text
                    .decode()
                    .map_err(|_| HarnessError::invalid("Invalid bundle metadata text."))?
                    .trim()
                    .is_empty()
                {
                    return Err(HarnessError::invalid(
                        "Unexpected text outside bundle metadata.",
                    ));
                }
            }
            Event::Empty(_) if depth <= 1 => {
                return Err(HarnessError::invalid(
                    "Unexpected empty bundle metadata root.",
                ));
            }
            Event::GeneralRef(_) if depth == 0 => {
                return Err(HarnessError::invalid(
                    "Unexpected reference outside bundle metadata.",
                ));
            }
            Event::Decl(_) | Event::DocType(_) if root_seen => {
                return Err(HarnessError::invalid(
                    "Unexpected bundle metadata declaration.",
                ));
            }
            Event::End(_) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| HarnessError::invalid("Invalid application bundle metadata."))?
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !root_seen || depth != 0 || expecting_identifier || identifier.as_deref() != Some(BUNDLE_ID)
    {
        return Err(HarnessError::forbidden(
            "This bundle belongs to another application; it was preserved.",
        ));
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn remove_registry() -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{HKEY_CURRENT_USER, RegDeleteKeyW},
    };
    let key = std::ffi::OsStr::new(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\AIAccelerator",
    )
    .encode_wide()
    .chain(Some(0))
    .collect::<Vec<_>>();
    let status = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, key.as_ptr()) };
    if ![ERROR_SUCCESS, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND].contains(&status) {
        return Err(HarnessError::unavailable(
            "Windows application registration could not be removed; the installation record was preserved.",
        ));
    }
    Ok(())
}
#[cfg(not(windows))]
pub(super) fn remove_registry() -> Result<()> {
    Ok(())
}

#[cfg(windows)]
pub(super) fn stage_shortcut(source: &Path, config: &Path, inputs: &mut Inputs) -> Result<Vec<u8>> {
    inputs.check()?;
    let bytes = super::windows_shortcut::stage(
        &inputs.worker,
        &inputs.executable,
        source,
        &config.join("ai-accelerator.ico"),
    )?;
    inputs.check()?;
    Ok(bytes)
}

#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use super::*;
    fn sandbox() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }
    fn file(root: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = root.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
    fn plan(record: &Path, target: &Path) -> Uninstall {
        Uninstall {
            record: Target::admit(record, RECORD_LIMIT).unwrap(),
            targets: vec![Target::admit(target, OUTPUT_LIMIT).unwrap()],
        }
    }
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn actionable_external_cleanup_failure_preserves_record_and_missing_target_retry() {
        let fixture = sandbox();
        let root = std::fs::canonicalize(fixture.path()).unwrap();
        let record = file(
            &root,
            "app.json",
            b"record retained until all effects complete",
        );
        let target = file(&root, "ai-accelerator.desktop", b"owned desktop image");
        let sentinel = file(&root, "unrelated", b"preserve unrelated data");
        let before = std::fs::read(&record).unwrap();
        let error = plan(&record, &target)
            .finish(|| Err(HarnessError::unavailable("injected registry denial")))
            .unwrap_err();
        assert!(error.message.contains("registry denial"));
        assert!(!target.exists());
        assert_eq!(std::fs::read(&record).unwrap(), before);
        let removed = plan(&record, &target).finish(|| Ok(())).unwrap();
        assert!(removed.is_empty());
        assert!(!record.exists());
        assert_eq!(
            std::fs::read(&sentinel).unwrap(),
            b"preserve unrelated data"
        );
    }
    #[cfg(unix)]
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn same_byte_replacement_is_preserved_and_record_is_kept() {
        let fixture = sandbox();
        let root = std::fs::canonicalize(fixture.path()).unwrap();
        let record = file(&root, "app.json", b"unchanged record");
        let target = file(&root, "ai-accelerator.desktop", b"same bytes");
        let pending = plan(&record, &target);
        std::fs::rename(&target, root.join("retained owned inode")).unwrap();
        std::fs::write(&target, b"same bytes").unwrap();
        assert!(
            pending
                .finish(|| panic!("external cleanup must not run"))
                .is_err()
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"same bytes");
        assert_eq!(std::fs::read(&record).unwrap(), b"unchanged record");
    }
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn pending_quarantine_refuses_a_blind_missing_target_retry() {
        let fixture = sandbox();
        let root = std::fs::canonicalize(fixture.path()).unwrap();
        let target = root.join("ai-accelerator.desktop");
        let quarantine = file(
            &root,
            ".ai-accelerator.desktop.known-attempt.removed",
            b"retained original bytes",
        );
        let mut admitted = Target::admit(&target, OUTPUT_LIMIT).unwrap();
        assert!(admitted.refuse_pending_removal().is_err());
        assert_eq!(
            std::fs::read(quarantine).unwrap(),
            b"retained original bytes"
        );
    }
    #[cfg(unix)]
    #[test]
    fn missing_native_parent_is_an_absence_witness_without_creating_directories() {
        let fixture = sandbox();
        let root = std::fs::canonicalize(fixture.path()).unwrap();
        let missing = root.join(r"missing\literal data");
        let target = missing.join("applications/ai-accelerator.desktop");
        let mut admitted = Target::admit(&target, OUTPUT_LIMIT).unwrap();
        admitted.refuse_pending_removal().unwrap();
        assert!(!admitted.remove(OUTPUT_LIMIT).unwrap());
        assert!(!missing.exists());
        assert!(!root.join("missing").exists());
        std::fs::create_dir(&missing).unwrap();
        let sentinel = missing.join("unrelated");
        std::fs::write(&sentinel, b"retain the new directory and its data").unwrap();
        assert!(
            admitted.remove(OUTPUT_LIMIT).is_err(),
            "newly appeared native parent was accepted as absent"
        );
        assert_eq!(
            std::fs::read(&sentinel).unwrap(),
            b"retain the new directory and its data"
        );
    }
}
