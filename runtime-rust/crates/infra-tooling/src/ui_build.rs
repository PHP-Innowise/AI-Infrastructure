//! Compile the browser's Rust logic and publish a verified wasm-bindgen ABI pair.
use crate::{common, process};
use infra_core::{Result, RuntimeError, bank::publication::PublicationFiles};
use serde_json::{Value, json};
#[cfg(windows)]
use std::ffi::OsStr;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
type Image = (u64, u64, u32, u64, u64, i128, i128);
type DirectoryImages = BTreeMap<String, (u64, u64, u32)>;
const TARGET: &str = "wasm32-unknown-unknown";
const CARGO_PROFILE: &str = "wasm-ui";
const UI: &str = "runtime-rust/crates/infra-ui";
const JS: &str = "runtime-rust/target/wasm-ui/infra_ui.js";
const WASM: &str = "runtime-rust/target/wasm-ui/infra_ui_bg.wasm";
const MANIFEST: &str = "runtime-rust/target/wasm-ui/abi-manifest.json";
const JS_LIMIT: usize = 8 * 1024 * 1024;
const WASM_LIMIT: usize = 64 * 1024 * 1024;
const MANIFEST_LIMIT: usize = 32 * 1024;
const SOURCE_LIMIT: usize = 32 * 1024 * 1024;
const LOG_LIMIT: usize = 4 * 1024 * 1024;
const SOURCE_COUNT: usize = 1000;
fn bound(request: &Value, key: &str, default: u64, minimum: u64, maximum: u64) -> Result<u64> {
    match request.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(value) => value
            .as_u64()
            .filter(|value| *value >= minimum && *value <= maximum)
            .ok_or_else(|| {
                RuntimeError::invalid(format!(
                    "{key} must be an integer from {minimum} through {maximum}"
                ))
            }),
    }
}
fn boolean(request: &Value, key: &str, default: bool) -> Result<bool> {
    match request.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| RuntimeError::invalid(format!("{key} must be boolean"))),
    }
}
fn read_leb128_u32(bytes: &[u8], pos: &mut usize) -> Result<u32> {
    let mut value: u32 = 0;
    let mut shift = 0u32;
    for _ in 0..5 {
        let byte = *bytes
            .get(*pos)
            .ok_or_else(|| RuntimeError::invalid("Truncated Wasm LEB128 integer"))?;
        *pos += 1;
        let low = u32::from(byte & 0x7f);
        if shift == 28 && (byte & 0x70) != 0 {
            return Err(RuntimeError::invalid("Overflow in Wasm LEB128 u32"));
        }
        value |= low << shift;
        if (byte & 0x80) == 0 {
            return Ok(value);
        }
        shift += 7;
    }
    Err(RuntimeError::invalid("Overlong Wasm LEB128 u32"))
}
fn is_debug_custom_section(payload: &[u8]) -> Result<bool> {
    let mut pos = 0usize;
    let name_len = read_leb128_u32(payload, &mut pos)? as usize;
    let end = pos
        .checked_add(name_len)
        .filter(|&end| end <= payload.len())
        .ok_or_else(|| RuntimeError::invalid("Truncated Wasm custom section name"))?;
    let name = std::str::from_utf8(&payload[pos..end]);
    Ok(match name {
        Ok("name" | "sourceMappingURL" | "external_debug_info") => true,
        Ok(other) => other.starts_with(".debug_"),
        Err(_) => false,
    })
}
fn strip_wasm_debug_sections(wasm: &[u8]) -> Result<(Vec<u8>, usize)> {
    if wasm.get(..8) != Some(&b"\0asm\x01\0\0\0"[..]) {
        return Err(RuntimeError::invalid(
            "Input is not a Wasm version 1 module",
        ));
    }
    let mut stripped = Vec::with_capacity(wasm.len());
    stripped.extend_from_slice(&wasm[..8]);
    let mut removed_bytes = 0usize;
    let mut pos = 8usize;
    while pos < wasm.len() {
        let section_start = pos;
        let section_id = wasm[pos];
        pos += 1;
        let section_len = read_leb128_u32(wasm, &mut pos)? as usize;
        let section_end = pos
            .checked_add(section_len)
            .filter(|&end| end <= wasm.len())
            .ok_or_else(|| RuntimeError::invalid("Truncated Wasm section payload"))?;
        if section_id == 0 && is_debug_custom_section(&wasm[pos..section_end])? {
            removed_bytes += section_end - section_start;
        } else {
            stripped.extend_from_slice(&wasm[section_start..section_end]);
        }
        pos = section_end;
    }
    Ok((stripped, removed_bytes))
}
fn wasm_opt_args(output: &Path, input: &Path, with_features: bool) -> Vec<OsString> {
    let mut args = vec![OsString::from("-Oz"), OsString::from("--strip-debug")];
    if with_features {
        for flag in [
            "--enable-bulk-memory",
            "--enable-nontrapping-float-to-int",
            "--enable-sign-ext",
            "--enable-mutable-globals",
        ] {
            args.push(OsString::from(flag));
        }
    }
    args.push(OsString::from("-o"));
    args.push(output.as_os_str().to_os_string());
    args.push(input.as_os_str().to_os_string());
    args
}
struct WasmOptimizationOutcome {
    wasm: Vec<u8>,
    wasm_opt_applied: bool,
    wasm_opt_flags: Vec<String>,
    bytes_before_opt: usize,
    bytes_after_opt: usize,
    bytes_saved: usize,
    debug_bytes_stripped: usize,
    warnings: Vec<String>,
}
fn apply_wasm_optimization(
    wasm: Vec<u8>,
    root: &Path,
    deadline: Instant,
    environment: &BTreeMap<OsString, Option<OsString>>,
    stage: &mut Stage,
    source: Option<&mut SourceSnapshot>,
    wasm_opt: Option<&mut Tool>,
) -> Result<WasmOptimizationOutcome> {
    let bytes_before_opt = wasm.len();
    let mut wasm = wasm;
    let mut wasm_opt_applied = false;
    let mut wasm_opt_flags: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    if let Some(optimizer) = wasm_opt {
        let optimized_rel = "bindgen/infra_ui_bg.opt.wasm";
        let opt_out = stage.path.join(optimized_rel);
        let opt_in = stage.path.join("bindgen/infra_ui_bg.wasm");
        let mut opt_args = wasm_opt_args(&opt_out, &opt_in, true);
        if optimizer
            .run(
                &opt_args,
                root,
                deadline,
                environment,
                "Wasm size optimization (wasm-opt -Oz --strip-debug)",
            )
            .is_err()
        {
            let fallback_args = wasm_opt_args(&opt_out, &opt_in, false);
            optimizer.run(
                &fallback_args,
                root,
                deadline,
                environment,
                "Wasm size optimization (wasm-opt -Oz --strip-debug)",
            )?;
            opt_args = fallback_args;
        }
        wasm_opt_flags = opt_args
            .iter()
            .take_while(|arg| arg.as_os_str() != "-o")
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        if let Some(snapshot) = source {
            snapshot.verify()?;
        }
        stage.verify()?;
        let (opt_wasm, opt_image) = stage
            .files
            .capture_source(optimized_rel, WASM_LIMIT)?
            .ok_or_else(|| RuntimeError::unavailable("wasm-opt -Oz produced no output module"))?;
        if opt_wasm.get(..8) != Some(&b"\0asm\x01\0\0\0"[..]) {
            return Err(RuntimeError::invalid(
                "wasm-opt output is not a Wasm version 1 module",
            ));
        }
        if stage.files.image_identity(optimized_rel)? != Some(opt_image) {
            return Err(RuntimeError::conflict(
                "Optimized Wasm module changed during capture",
            ));
        }
        optimizer.verify()?;
        wasm = opt_wasm;
        wasm_opt_applied = true;
    } else {
        warnings.push(
            "wasm-opt was not found on PATH; skipped Binaryen -Oz pass and applied custom debug/name section stripping only."
                .to_owned(),
        );
    }
    let (wasm, debug_bytes_stripped) = strip_wasm_debug_sections(&wasm)?;
    let bytes_after_opt = wasm.len();
    let bytes_saved = bytes_before_opt.saturating_sub(bytes_after_opt);
    Ok(WasmOptimizationOutcome {
        wasm,
        wasm_opt_applied,
        wasm_opt_flags,
        bytes_before_opt,
        bytes_after_opt,
        bytes_saved,
        debug_bytes_stripped,
        warnings,
    })
}
fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| RuntimeError::unavailable("UI build time limit exceeded"))
}
fn text(bytes: &[u8], label: &str) -> Result<String> {
    std::str::from_utf8(bytes)
        .map(str::to_string)
        .map_err(|_| RuntimeError::invalid(format!("{label} must be UTF-8")))
}
fn document(bytes: &[u8], label: &str) -> Result<toml_edit::DocumentMut> {
    text(bytes, label)?
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| RuntimeError::invalid(format!("Invalid {label}: {error}")))
}
struct Tool {
    selected: PathBuf,
    canonical: PathBuf,
    files: PublicationFiles,
    file: std::fs::File,
    image: infra_core::fs::NativeIdentity,
}
impl Tool {
    fn open(candidate: &Path) -> Result<Self> {
        let selected = infra_core::fs::absolute(candidate)?;
        let canonical = infra_core::fs::canonicalize(&selected).map_err(|_| {
            RuntimeError::unavailable(format!(
                "Native build executable is unavailable: {}",
                selected.display()
            ))
        })?;
        #[cfg(windows)]
        if !selected
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|extension| {
                ["exe", "com"]
                    .iter()
                    .any(|allowed| extension.eq_ignore_ascii_case(allowed))
            })
        {
            return Err(RuntimeError::policy(
                "UI build tools must be native executables, not command shims",
            ));
        }
        let parent = canonical
            .parent()
            .ok_or_else(|| RuntimeError::invalid("Build executable has no parent"))?;
        let files = PublicationFiles::new(parent)?;
        let image = infra_core::fs::native_image(&canonical)?
            .ok_or_else(|| RuntimeError::unavailable("Build executable disappeared"))?;
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x00200000);
        }
        let file = options.open(&canonical)?;
        if !file.metadata()?.is_file()
            || image.reparse_tag != 0
            || infra_core::fs::native_identity::handle_identity(&file)? != image
        {
            return Err(RuntimeError::policy(
                "Build executable must resolve to an ordinary native file",
            ));
        }
        #[cfg(unix)]
        if image.kind & 0o111 == 0 {
            return Err(RuntimeError::policy("Build executable is not executable"));
        }
        // Rustup proxies may be hard links. Retain their exact native image rather
        // than imposing the single-link policy used for mutable published assets.
        let tool = Self {
            selected,
            canonical,
            files,
            file,
            image,
        };
        tool.check()?;
        Ok(tool)
    }
    fn find(root: &Path, request: &Value, key: &str, default: &str) -> Result<Self> {
        Self::find_optional(root, request, key, default)?.ok_or_else(|| {
            RuntimeError::unavailable(format!(
                "Native {default} executable is unavailable; provide --{key}"
            ))
        })
    }
    fn find_optional(
        root: &Path,
        request: &Value,
        key: &str,
        default: &str,
    ) -> Result<Option<Self>> {
        let explicit = match request.get(key) {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_str()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| {
                        RuntimeError::invalid(format!("{key} must name one native executable"))
                    })?,
            ),
        };
        let requested = Path::new(explicit.unwrap_or(default));
        if requested.is_absolute() || requested.components().count() > 1 {
            return Self::open(&root.join(requested)).map(Some);
        }
        let Some(search) = std::env::var_os("PATH") else {
            return if explicit.is_some() {
                Err(RuntimeError::unavailable(format!(
                    "Native {default} executable is unavailable; provide --{key}"
                )))
            } else {
                Ok(None)
            };
        };
        #[cfg(windows)]
        let names = if requested.extension().is_some() {
            vec![requested.as_os_str().to_os_string()]
        } else {
            vec![
                OsString::from(format!("{}.exe", requested.display())),
                OsString::from(format!("{}.com", requested.display())),
            ]
        };
        #[cfg(not(windows))]
        let names = vec![requested.as_os_str().to_os_string()];
        for directory in std::env::split_paths(&search) {
            if directory.as_os_str().is_empty() {
                continue;
            }
            for name in &names {
                let candidate = directory.join(name);
                if candidate.is_file() {
                    return Self::open(&candidate).map(Some);
                }
            }
        }
        if explicit.is_some() {
            return Err(RuntimeError::unavailable(format!(
                "Native {default} executable is unavailable; provide --{key}"
            )));
        }
        Ok(None)
    }
    fn check(&self) -> Result<()> {
        self.files.check()?;
        if infra_core::fs::canonicalize(&self.selected)? != self.canonical {
            return Err(RuntimeError::conflict(
                "Build executable resolution changed",
            ));
        }
        if infra_core::fs::native_image(&self.canonical)?.as_ref() != Some(&self.image)
            || infra_core::fs::native_identity::handle_identity(&self.file)? != self.image
        {
            return Err(RuntimeError::conflict("Build executable image changed"));
        }
        self.files.check()
    }
    fn verify(&mut self) -> Result<()> {
        self.check()
    }
    fn run(
        &mut self,
        args: &[OsString],
        cwd: &Path,
        deadline: Instant,
        environment: &BTreeMap<OsString, Option<OsString>>,
        label: &str,
    ) -> Result<process::ProcessOutput> {
        self.verify()?;
        let output = process::run_with_env(
            self.selected.as_os_str(),
            args,
            cwd,
            None,
            remaining(deadline)?,
            LOG_LIMIT,
            environment,
        );
        self.verify()?;
        let output = output?;
        if output.code != 0 {
            let diagnostics = if output.stderr.is_empty() {
                &output.stdout
            } else {
                &output.stderr
            };
            let diagnostic = String::from_utf8_lossy(diagnostics);
            let suffix = diagnostic
                .chars()
                .rev()
                .take(12000)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>();
            return Err(RuntimeError::unavailable(format!(
                "{label} failed with exit code {}:\n{suffix}",
                output.code
            )));
        }
        Ok(output)
    }
}
struct SourceSnapshot {
    files: PublicationFiles,
    images: BTreeMap<String, Image>,
    names: BTreeMap<String, Vec<String>>,
    directories: DirectoryImages,
    bytes: BTreeMap<String, Vec<u8>>,
    total: usize,
}
impl SourceSnapshot {
    fn capture(root: &Path) -> Result<Self> {
        let files = PublicationFiles::new(root)?;
        let mut snapshot = Self {
            files,
            images: BTreeMap::new(),
            names: BTreeMap::new(),
            directories: BTreeMap::new(),
            bytes: BTreeMap::new(),
            total: 0,
        };
        snapshot.file("runtime-rust/Cargo.toml")?;
        snapshot.file("runtime-rust/Cargo.lock")?;
        snapshot.tree(UI, 0)?;
        snapshot.directories = snapshot.files.directory_identities()?;
        snapshot.verify()?;
        Ok(snapshot)
    }
    fn file(&mut self, name: &str) -> Result<()> {
        if self.images.len() >= SOURCE_COUNT {
            return Err(RuntimeError::policy(
                "Rust UI source file count exceeds its bound",
            ));
        }
        let (bytes, image) = self
            .files
            .capture_source(name, SOURCE_LIMIT)?
            .ok_or_else(|| {
                RuntimeError::not_found(format!("Required Rust UI source is absent: {name}"))
            })?;
        self.total = self
            .total
            .checked_add(bytes.len())
            .filter(|total| *total <= SOURCE_LIMIT)
            .ok_or_else(|| {
                RuntimeError::policy("Rust UI sources exceed their combined byte bound")
            })?;
        self.images.insert(name.into(), image);
        self.bytes.insert(name.into(), bytes);
        Ok(())
    }
    fn tree(&mut self, directory: &str, depth: usize) -> Result<()> {
        if depth > 32 {
            return Err(RuntimeError::policy(
                "Rust UI source nesting exceeds its bound",
            ));
        }
        let mut names = self.files.archive_entries(directory, SOURCE_COUNT)?;
        names.sort();
        self.names.insert(directory.into(), names.clone());
        for name in names {
            if [".git", "target"].contains(&name.as_str()) {
                return Err(RuntimeError::policy(
                    "Rust UI source tree contains a nested repository or build tree",
                ));
            }
            let child = format!("{directory}/{name}");
            match self.files.image_identity(&child) {
                Ok(Some(_)) => self.file(&child)?,
                Ok(None) => {
                    return Err(RuntimeError::conflict(
                        "Rust UI source disappeared during capture",
                    ));
                }
                Err(error) => {
                    if self.files.archive_directory_exists(&child)? {
                        self.tree(&child, depth + 1)?;
                    } else {
                        return Err(error);
                    }
                }
            }
        }
        Ok(())
    }
    fn verify(&mut self) -> Result<()> {
        if !self.directories.is_empty() {
            self.files.check_directory_identities(&self.directories)?;
        }
        for (name, image) in &self.images {
            if self.files.image_identity(name)? != Some(*image) {
                return Err(RuntimeError::conflict(format!(
                    "Rust UI source changed during build: {name}"
                )));
            }
        }
        for (directory, expected) in &self.names {
            let mut actual = self.files.archive_entries(directory, SOURCE_COUNT)?;
            actual.sort();
            if &actual != expected {
                return Err(RuntimeError::conflict(format!(
                    "Rust UI source membership changed during build: {directory}"
                )));
            }
        }
        self.files.check()
    }
    fn bytes(&self, name: &str) -> Result<&[u8]> {
        self.bytes.get(name).map(Vec::as_slice).ok_or_else(|| {
            RuntimeError::not_found(format!("Rust UI source was not captured: {name}"))
        })
    }
    fn digest(&self) -> String {
        let mut bytes = Vec::new();
        for (name, body) in &self.bytes {
            bytes.extend_from_slice(&(name.len() as u64).to_le_bytes());
            bytes.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(common::sha256(body).as_bytes());
        }
        common::sha256(&bytes)
    }
}
struct Stage {
    outer: PublicationFiles,
    files: PublicationFiles,
    path: PathBuf,
    name: String,
    outer_directories: DirectoryImages,
    closed: bool,
}
impl Stage {
    fn new() -> Result<Self> {
        let temporary = infra_core::fs::canonicalize(std::env::temp_dir())?;
        let mut outer = PublicationFiles::new(&temporary)?;
        let name = format!("infra-ui-build-{}", uuid::Uuid::new_v4().simple());
        outer.archive_create_directory(&name, true)?;
        let outer_directories = outer.directory_identities()?;
        let path = temporary.join(&name);
        let files = PublicationFiles::new(&path)?;
        if outer_directories.get(&name).copied() != Some(files.root_identity()) {
            return Err(RuntimeError::conflict(
                "Private UI build directory changed during admission",
            ));
        }
        let stage = Self {
            outer,
            files,
            path,
            name,
            outer_directories,
            closed: false,
        };
        stage.check()?;
        Ok(stage)
    }
    fn check(&self) -> Result<()> {
        self.outer.check()?;
        self.files.check()
    }
    fn verify(&mut self) -> Result<()> {
        self.outer
            .check_directory_identities(&self.outer_directories)?;
        self.files.check()
    }
    fn cleanup(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.verify()?;
        let mut count = 0;
        remove_children(&mut self.files, "", 0, &mut count)?;
        if !self.outer.archive_remove_empty_directory(&self.name)? {
            return Err(RuntimeError::unavailable(
                "Private UI build directory could not be emptied",
            ));
        }
        self.closed = true;
        Ok(())
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        if !self.closed {
            let _ = self.cleanup();
        }
    }
}
fn remove_children(
    files: &mut PublicationFiles,
    directory: &str,
    depth: usize,
    count: &mut usize,
) -> Result<()> {
    if depth > 64 {
        return Err(RuntimeError::policy(
            "Private UI build cleanup depth exceeds its bound",
        ));
    }
    for name in files.archive_entries(directory, 100_000)? {
        *count = count
            .checked_add(1)
            .ok_or_else(|| RuntimeError::policy("Private UI build cleanup count overflow"))?;
        if *count > 100_000 {
            return Err(RuntimeError::policy(
                "Private UI build cleanup entry bound exceeded",
            ));
        }
        let child = if directory.is_empty() {
            name
        } else {
            format!("{directory}/{name}")
        };
        match files.archive_directory_exists(&child) {
            Ok(true) => {
                remove_children(files, &child, depth + 1, count)?;
                if !files.archive_remove_empty_directory(&child)? {
                    return Err(RuntimeError::conflict(
                        "Private UI build cleanup encountered a changed directory",
                    ));
                }
            }
            Ok(false) => {}
            Err(_) => {
                files.archive_remove_entry(&child)?;
            }
        }
    }
    Ok(())
}
#[derive(Clone)]
struct Before {
    bytes: Option<Vec<u8>>,
    mode: Option<u32>,
    image: Option<Image>,
}
fn before(files: &mut PublicationFiles, name: &str, limit: usize) -> Result<Before> {
    let image = files.image_identity(name)?;
    let bytes = files.read(name, limit)?;
    let mode = files.read_mode(name)?;
    if files.image_identity(name)? != image
        || bytes.is_some() != image.is_some()
        || mode.is_some() != image.is_some()
    {
        return Err(RuntimeError::conflict("UI asset changed during capture"));
    }
    Ok(Before { bytes, mode, image })
}
fn unchanged(
    files: &mut PublicationFiles,
    images: &BTreeMap<String, Before>,
    directories: &DirectoryImages,
) -> Result<()> {
    files.check_directory_identities(directories)?;
    for (name, before) in images {
        if files.image_identity(name)? != before.image {
            return Err(RuntimeError::conflict(format!(
                "UI asset changed before publication: {name}"
            )));
        }
    }
    files.check()
}
fn publish(
    files: &mut PublicationFiles,
    javascript: &[u8],
    wasm: &[u8],
    manifest: &[u8],
) -> Result<()> {
    let outputs = [
        (JS, javascript, JS_LIMIT),
        (WASM, wasm, WASM_LIMIT),
        (MANIFEST, manifest, MANIFEST_LIMIT),
    ];
    let mut captured = BTreeMap::new();
    for (name, _, limit) in &outputs {
        captured.insert((*name).into(), before(files, name, *limit)?);
    }
    let mut directories = files.directory_identities()?;
    for (name, bytes, limit) in outputs {
        unchanged(files, &captured, &directories)?;
        let previous = captured
            .get(name)
            .ok_or_else(|| RuntimeError::unavailable("UI publication snapshot is incomplete"))?;
        if previous.bytes.as_deref() != Some(bytes) || previous.mode != Some(0o644) {
            files.write_expected_mode_checked(
                name,
                bytes,
                previous.bytes.as_deref(),
                previous.mode,
                0o644,
                limit,
            )?;
        }
        captured.insert(name.into(), before(files, name, limit)?);
        directories = files.directory_identities()?;
    }
    unchanged(files, &captured, &directories)?;
    for (name, expected, _) in outputs {
        if captured[name].bytes.as_deref() != Some(expected) {
            return Err(RuntimeError::conflict(
                "Published UI artifact bytes differ from the immutable build output",
            ));
        }
    }
    Ok(())
}
fn wasm_bindgen_version(snapshot: &SourceSnapshot) -> Result<String> {
    let document = document(
        snapshot.bytes("runtime-rust/Cargo.lock")?,
        "workspace Cargo.lock",
    )?;
    let packages = document
        .get("package")
        .and_then(toml_edit::Item::as_array_of_tables)
        .ok_or_else(|| RuntimeError::invalid("Workspace Cargo.lock has no package records"))?;
    let mut versions = packages
        .iter()
        .filter(|package| {
            package.get("name").and_then(toml_edit::Item::as_str) == Some("wasm-bindgen")
        })
        .filter_map(|package| package.get("version").and_then(toml_edit::Item::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();
    versions.sort();
    versions.dedup();
    if versions.len() != 1 {
        return Err(RuntimeError::invalid(
            "UI build requires one resolved wasm-bindgen version in Cargo.lock; resolve the workspace dependencies before building the UI",
        ));
    }
    Ok(versions.remove(0))
}
/// Digest of the sources a UI build under `root` would compile, as the build
/// records it in `abi-manifest.json`.
pub fn source_sha256(root: &Path) -> Result<String> {
    Ok(SourceSnapshot::capture(&infra_core::fs::absolute(root)?)?.digest())
}
/// Build after the workspace has been composed and its lockfile resolved. The
/// default is offline and locked; tool installation and target installation are
/// explicit operator actions, never automatic fallbacks in the browser server.
pub fn build(root: &Path, request: &Value) -> Result<Value> {
    if !request.is_object() {
        return Err(RuntimeError::invalid("UI build request must be an object"));
    }
    let root = infra_core::fs::absolute(root)?;
    let seconds = bound(request, "timeout_seconds", 1800, 1, 7200)?;
    let jobs = bound(request, "jobs", 2, 1, 32)?;
    let offline = boolean(request, "offline", true)?;
    let require_wasm_opt = boolean(request, "require_wasm_opt", false)?;
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(seconds))
        .ok_or_else(|| RuntimeError::invalid("UI build deadline cannot be represented"))?;
    let mut source = SourceSnapshot::capture(&root)?;
    let manifest = document(
        source.bytes(&format!("{UI}/Cargo.toml"))?,
        "infra-ui Cargo.toml",
    )?;
    if manifest
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml_edit::Item::as_str)
        != Some("infra-ui")
    {
        return Err(RuntimeError::invalid(
            "The browser package must be named infra-ui",
        ));
    }
    let version = wasm_bindgen_version(&source)?;
    let mut cargo = Tool::find(&root, request, "cargo", "cargo")?;
    let mut bindgen = Tool::find(&root, request, "wasm_bindgen", "wasm-bindgen")?;
    let mut wasm_opt = if require_wasm_opt {
        Some(Tool::find(&root, request, "wasm_opt", "wasm-opt")?)
    } else {
        Tool::find_optional(&root, request, "wasm_opt", "wasm-opt")?
    };
    let mut environment = BTreeMap::<OsString, Option<OsString>>::new();
    environment.insert("CARGO_TARGET_DIR".into(), None);
    for key in [
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
    ] {
        environment.insert(key.into(), Some(OsString::new()));
    }
    environment.insert("CARGO_INCREMENTAL".into(), Some("0".into()));
    environment.insert(
        "CARGO_NET_OFFLINE".into(),
        Some(if offline { "true" } else { "false" }.into()),
    );
    let found = bindgen.run(
        &[OsString::from("--version")],
        &root,
        deadline,
        &environment,
        "wasm-bindgen version probe",
    )?;
    let found = text(&found.stdout, "wasm-bindgen version output")?;
    let mut words = found.split_whitespace();
    if words.next() != Some("wasm-bindgen") || words.next() != Some(version.as_str()) {
        return Err(RuntimeError::unavailable(format!(
            "The native wasm-bindgen CLI must match the locked crate version {version}"
        )));
    }
    source.verify()?;
    let mut stage = Stage::new()?;
    let result = (|| -> Result<Value> {
        let target_directory = stage.path.join("cargo-target");
        let mut arguments = vec![
            OsString::from("build"),
            OsString::from("--manifest-path"),
            root.join("runtime-rust/Cargo.toml").into_os_string(),
            OsString::from("--package"),
            OsString::from("infra-ui"),
            OsString::from("--target"),
            OsString::from(TARGET),
            OsString::from("--profile"),
            OsString::from(CARGO_PROFILE),
            OsString::from("--locked"),
            OsString::from("--target-dir"),
            target_directory.clone().into_os_string(),
            OsString::from("--jobs"),
            jobs.to_string().into(),
        ];
        if offline {
            arguments.push("--offline".into());
        }
        cargo.run(
            &arguments,
            &root,
            deadline,
            &environment,
            "Rust UI compilation",
        )?;
        source.verify()?;
        stage.verify()?;
        let compiled = format!("cargo-target/{TARGET}/{CARGO_PROFILE}/infra_ui.wasm");
        let (raw, _) = stage
            .files
            .capture_source(&compiled, WASM_LIMIT)?
            .ok_or_else(|| {
                RuntimeError::unavailable("Rust UI compilation produced no declared Wasm artifact")
            })?;
        if raw.get(..8) != Some(&b"\0asm\x01\0\0\0"[..]) {
            return Err(RuntimeError::invalid(
                "Rust UI compiler output is not a Wasm version 1 module",
            ));
        }
        // Cargo can hard-link its final artifact to deps/. Freeze the captured
        // bytes into a private single-link input before the ABI tool reads it.
        let input = "bindgen-input.wasm";
        stage
            .files
            .write_expected_mode_checked(input, &raw, None, None, 0o600, WASM_LIMIT)?;
        let input_image = stage
            .files
            .image_identity(input)?
            .ok_or_else(|| RuntimeError::unavailable("Frozen Wasm ABI input disappeared"))?;
        stage.files.archive_create_directory("bindgen", true)?;
        let arguments = vec![
            stage.path.join(input).into_os_string(),
            OsString::from("--target"),
            OsString::from("web"),
            OsString::from("--no-typescript"),
            OsString::from("--out-name"),
            OsString::from("infra_ui"),
            OsString::from("--out-dir"),
            stage.path.join("bindgen").into_os_string(),
        ];
        bindgen.run(
            &arguments,
            &root,
            deadline,
            &environment,
            "Wasm ABI generation",
        )?;
        source.verify()?;
        stage.verify()?;
        if stage.files.image_identity(input)? != Some(input_image) {
            return Err(RuntimeError::conflict(
                "Frozen Wasm input changed during ABI generation",
            ));
        }
        let (javascript, js_image) = stage
            .files
            .capture_source("bindgen/infra_ui.js", JS_LIMIT)?
            .ok_or_else(|| {
                RuntimeError::unavailable("Wasm ABI generator produced no JavaScript module")
            })?;
        let (wasm, wasm_image) = stage
            .files
            .capture_source("bindgen/infra_ui_bg.wasm", WASM_LIMIT)?
            .ok_or_else(|| {
                RuntimeError::unavailable("Wasm ABI generator produced no Wasm module")
            })?;
        if javascript.is_empty()
            || text(&javascript, "Generated Wasm ABI module")?
                .trim()
                .is_empty()
        {
            return Err(RuntimeError::invalid("Generated Wasm ABI module is empty"));
        }
        if wasm.get(..8) != Some(&b"\0asm\x01\0\0\0"[..]) {
            return Err(RuntimeError::invalid(
                "Generated Wasm ABI output is not a Wasm version 1 module",
            ));
        }
        if stage.files.image_identity("bindgen/infra_ui.js")? != Some(js_image)
            || stage.files.image_identity("bindgen/infra_ui_bg.wasm")? != Some(wasm_image)
        {
            return Err(RuntimeError::conflict(
                "Generated Wasm ABI pair changed during capture",
            ));
        }
        let wasm_opt_selected = wasm_opt.as_ref().map(|tool| tool.selected.clone());
        let optimization = apply_wasm_optimization(
            wasm,
            &root,
            deadline,
            &environment,
            &mut stage,
            Some(&mut source),
            wasm_opt.as_mut(),
        )?;
        let wasm = optimization.wasm;
        let wasm_opt_applied = optimization.wasm_opt_applied;
        let wasm_opt_flags = optimization.wasm_opt_flags;
        let wasm_bytes_before_opt = optimization.bytes_before_opt;
        let wasm_bytes_after_opt = optimization.bytes_after_opt;
        let wasm_bytes_saved = optimization.bytes_saved;
        let debug_bytes_stripped = optimization.debug_bytes_stripped;
        let warnings = optimization.warnings;
        let js_hash = common::sha256(&javascript);
        let wasm_hash = common::sha256(&wasm);
        // The source digest lets a Harness update tell whether this pair was
        // compiled from the sources the clone now has.
        let manifest = common::json_bytes(
            &json!({"schema_version":1,"js_sha256":js_hash,"wasm_sha256":wasm_hash,"source_sha256":source.digest()}),
        )?;
        source.verify()?;
        cargo.verify()?;
        bindgen.verify()?;
        publish(&mut source.files, &javascript, &wasm, &manifest)?;
        source.verify()?;
        stage.verify()?;
        let mut report = json!({
            "ok": true,
            "target": TARGET,
            "profile": CARGO_PROFILE,
            "wasm_opt_applied": wasm_opt_applied,
            "wasm_opt": wasm_opt_selected,
            "wasm_opt_flags": wasm_opt_flags,
            "wasm_bytes_before_opt": wasm_bytes_before_opt,
            "wasm_bytes_after_opt": wasm_bytes_after_opt,
            "wasm_bytes_saved": wasm_bytes_saved,
            "wasm_debug_bytes_stripped": debug_bytes_stripped,
            "offline": offline,
            "jobs": jobs,
            "wasm_bindgen_version": version,
            "cargo": cargo.selected,
            "wasm_bindgen": bindgen.selected,
            "source_sha256": source.digest(),
            "source_files": source.images.len(),
            "artifacts": [
                {"path": JS, "bytes": javascript.len(), "sha256": js_hash},
                {"path": WASM, "bytes": wasm.len(), "sha256": wasm_hash},
                {"path": MANIFEST, "bytes": manifest.len(), "sha256": common::sha256(&manifest)}
            ],
            "manifest_published_last": true,
            "elapsed_seconds": seconds as f64 - remaining(deadline).map(|remaining| remaining.as_secs_f64()).unwrap_or(0.),
        });
        if !warnings.is_empty() {
            report["warnings"] = json!(warnings);
        }
        Ok(report)
    })();
    let cleanup = stage.cleanup();
    match result {
        Ok(mut report) => {
            report["temporary_files_removed"] = json!(cleanup.is_ok());
            if cleanup.is_err() {
                let mut warnings = report
                    .get("warnings")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                warnings.push(json!(
                    "Private temporary build files could not be completely removed."
                ));
                report["warnings"] = Value::Array(warnings);
            }
            Ok(report)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_leb128_u32(mut value: u32) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                break;
            }
        }
        out
    }

    fn custom_section(name: &str, body: &[u8]) -> Vec<u8> {
        let mut payload = encode_leb128_u32(name.len() as u32);
        payload.extend_from_slice(name.as_bytes());
        payload.extend_from_slice(body);
        let mut section = vec![0u8];
        section.extend(encode_leb128_u32(payload.len() as u32));
        section.extend(payload);
        section
    }

    #[test]
    fn wasm_ui_profile_and_optional_wasm_opt_lookup() {
        assert_eq!(CARGO_PROFILE, "wasm-ui");
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .expect("workspace root");
        let cargo_toml = std::fs::read_to_string(root.join("runtime-rust/Cargo.toml")).unwrap();
        assert!(
            cargo_toml.contains("[profile.wasm-ui]"),
            "workspace Cargo.toml must define [profile.wasm-ui]"
        );
        assert!(
            cargo_toml.contains("lto = ") && cargo_toml.contains("strip = "),
            "workspace [profile.wasm-ui] must configure LTO and symbol stripping"
        );
        let missing = Tool::find_optional(
            &root,
            &json!({}),
            "wasm_opt",
            "definitely-missing-wasm-opt-binary-17d0858c",
        )
        .unwrap();
        assert!(missing.is_none());

        let explicit_missing = Tool::find_optional(
            &root,
            &json!({"wasm_opt": "/nonexistent/wasm-opt-bin"}),
            "wasm_opt",
            "wasm-opt",
        );
        assert!(explicit_missing.is_err());

        let args = wasm_opt_args(Path::new("/tmp/out.wasm"), Path::new("/tmp/in.wasm"), true);
        let arg_strs = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(arg_strs.contains(&"-Oz".to_string()));
        assert!(arg_strs.contains(&"--strip-debug".to_string()));
    }

    #[test]
    fn strip_wasm_debug_sections_drops_name_and_dwarf_and_preserves_abi_sections() {
        let mut wasm = b"\0asm\x01\0\0\0".to_vec();
        // Type section (id=1, len=1, body=[0])
        let type_section = vec![1u8, 1, 0];
        wasm.extend_from_slice(&type_section);
        // Preserved non-debug custom section
        let producers = custom_section("producers", b"rustc");
        wasm.extend_from_slice(&producers);
        // Debug custom sections that must be stripped
        let name_sec = custom_section("name", b"fn_symbols_table");
        let dwarf_sec = custom_section(".debug_info", b"dwarf_payload");
        let source_map_sec = custom_section("sourceMappingURL", b"ui.map");
        wasm.extend_from_slice(&name_sec);
        wasm.extend_from_slice(&dwarf_sec);
        wasm.extend_from_slice(&source_map_sec);

        let (stripped, removed) = strip_wasm_debug_sections(&wasm).unwrap();
        assert_eq!(
            removed,
            name_sec.len() + dwarf_sec.len() + source_map_sec.len()
        );
        let mut expected = b"\0asm\x01\0\0\0".to_vec();
        expected.extend_from_slice(&type_section);
        expected.extend_from_slice(&producers);
        assert_eq!(stripped, expected);
    }

    #[test]
    fn wasm_opt_invocation_invokes_oz_strip_debug_and_updates_manifest_bytes() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let temp = crate::temp::Temporary::new("wasm-opt-mock").unwrap();
            let root = infra_core::fs::canonicalize(temp.path()).unwrap();
            let bin = root.join("wasm-opt-mock");
            let args_log = root.join("args.txt");
            let script = format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{}\"\nout=\"\"\nprev=\"\"\nfor a in \"$@\"; do\n  if [ \"$prev\" = \"-o\" ]; then out=\"$a\"; fi\n  prev=\"$a\"\ndone\nprintf '\\000asm\\001\\000\\000\\000' > \"$out\"\n",
                args_log.display()
            );
            std::fs::write(&bin, script).unwrap();
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();

            let mut optimizer = Tool::open(&bin).unwrap();
            let mut stage = Stage::new().unwrap();
            stage
                .files
                .archive_create_directory("bindgen", true)
                .unwrap();
            let raw_wasm = [
                &b"\0asm\x01\0\0\0"[..],
                &custom_section("name", b"huge_symbol_table_bytes")[..],
            ]
            .concat();
            stage
                .files
                .write_expected_mode_checked(
                    "bindgen/infra_ui_bg.wasm",
                    &raw_wasm,
                    None,
                    None,
                    0o600,
                    WASM_LIMIT,
                )
                .unwrap();

            let deadline = Instant::now() + Duration::from_secs(30);
            let env = BTreeMap::new();
            let outcome = apply_wasm_optimization(
                raw_wasm.clone(),
                &root,
                deadline,
                &env,
                &mut stage,
                None,
                Some(&mut optimizer),
            )
            .unwrap();
            let recorded_args = std::fs::read_to_string(&args_log).unwrap();
            assert!(recorded_args.lines().any(|l| l == "-Oz"));
            assert!(recorded_args.lines().any(|l| l == "--strip-debug"));
            assert!(outcome.wasm_opt_applied);
            assert!(outcome.wasm_opt_flags.contains(&"-Oz".to_string()));
            assert!(
                outcome
                    .wasm_opt_flags
                    .contains(&"--strip-debug".to_string())
            );
            assert_eq!(outcome.bytes_before_opt, raw_wasm.len());
            assert_eq!(outcome.bytes_after_opt, outcome.wasm.len());
            assert!(outcome.bytes_saved > 0);
            assert!(outcome.wasm.len() < raw_wasm.len());
            assert!(outcome.warnings.is_empty());

            // Also verify none-optimizer fallback still strips custom debug/name sections cleanly.
            let stripped_only = apply_wasm_optimization(
                raw_wasm.clone(),
                &root,
                deadline,
                &env,
                &mut stage,
                None,
                None,
            )
            .unwrap();
            assert!(!stripped_only.wasm_opt_applied);
            assert_eq!(stripped_only.wasm.len(), b"\0asm\x01\0\0\0".len());
            assert!(stripped_only.debug_bytes_stripped > 0);
            assert!(!stripped_only.warnings.is_empty());
        }
    }

    #[test]
    fn require_wasm_opt_fails_when_missing() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .expect("workspace root");
        let err = build(
            &root,
            &json!({
                "require_wasm_opt": true,
                "wasm_opt": "/nonexistent/definitely-missing-wasm-opt-bin"
            }),
        )
        .unwrap_err();
        assert!(err.to_string().contains("unavailable") || err.to_string().contains("wasm-opt"));
    }
}
