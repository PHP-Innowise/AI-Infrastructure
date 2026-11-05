//! Native platform profiles and in-sandbox discovery boundary probe.
use crate::{HarnessError, Result, provider, store};
#[cfg(windows)]
use serde_json::Map;
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};
#[derive(Clone)]
pub struct Spec {
    pub workspace: PathBuf,
    pub target: PathBuf,
    pub protected: Vec<PathBuf>,
    pub readable: Vec<PathBuf>,
    pub accounts: Vec<PathBuf>,
    pub runtime: Vec<PathBuf>,
    pub codex: Option<PathBuf>,
    pub evidence: Option<PathBuf>,
}
fn canonical(path: &Path) -> Result<PathBuf> {
    let root = infra_core::fs::canonicalize(path)?;
    let meta = std::fs::symlink_metadata(path)?;
    if infra_core::fs::linked(&meta) {
        return Err(HarnessError::forbidden("Sandbox paths must not be links."));
    }
    Ok(root)
}
fn overlap(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}
fn arg(path: &Path) -> Result<String> {
    path.to_str()
        .map(String::from)
        .ok_or_else(|| HarnessError::invalid("Sandbox paths must be UTF-8."))
}
#[cfg(target_os = "macos")]
fn sbpl(path: &Path) -> Result<String> {
    Ok(format!(
        "\"{}\"",
        arg(path)?.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}
#[cfg(windows)]
fn toml(value: &Value) -> Result<String> {
    match value {
        Value::Object(map) => {
            let mut entries = Vec::new();
            for (k, v) in map {
                entries.push(format!("{}={}", json!(k), toml(v)?));
            }
            Ok(format!("{{{}}}", entries.join(",")))
        }
        Value::Array(array) => Ok(format!(
            "[{}]",
            array
                .iter()
                .map(toml)
                .collect::<Result<Vec<_>>>()?
                .join(",")
        )),
        _ => Ok(value.to_string()),
    }
}
fn prepared(spec: &Spec, discovery: bool) -> Result<Spec> {
    let mut spec = spec.clone();
    spec.runtime.push(
        trusted_helper(&spec.workspace)?
            .parent()
            .ok_or_else(|| HarnessError::invalid("Native helper parent missing."))?
            .to_path_buf(),
    );
    spec.workspace = canonical(&spec.workspace)?;
    spec.target = canonical(&spec.target)?;
    if !spec.workspace.is_dir() || !spec.target.is_dir() {
        return Err(HarnessError::invalid(
            "Sandbox workspace and target must be directories.",
        ));
    }
    spec.protected = spec
        .protected
        .iter()
        .map(|p| canonical(p))
        .collect::<Result<_>>()?;
    spec.accounts = spec
        .accounts
        .iter()
        .map(|p| canonical(p))
        .collect::<Result<_>>()?;
    spec.runtime = spec
        .runtime
        .iter()
        .map(|p| canonical(p))
        .collect::<Result<_>>()?;
    spec.readable = spec
        .readable
        .iter()
        .map(|p| canonical(p))
        .collect::<Result<_>>()?;
    if let Some(path) = &spec.evidence {
        spec.evidence = Some(canonical(path)?);
    }
    let control = spec
        .workspace
        .parent()
        .ok_or_else(|| HarnessError::invalid("Sandbox workspace needs a control parent."))?;
    for account in &spec.accounts {
        for protected in spec
            .protected
            .iter()
            .map(PathBuf::as_path)
            .chain([spec.target.as_path(), control])
        {
            if overlap(account, protected) {
                return Err(HarnessError::forbidden(
                    "Provider account state overlaps a protected directory.",
                ));
            }
        }
    }
    if discovery {
        for source in &spec.protected {
            for allowed in spec
                .runtime
                .iter()
                .chain(spec.accounts.iter())
                .chain(spec.readable.iter())
                .map(PathBuf::as_path)
                .chain([control])
            {
                if overlap(source, allowed) {
                    return Err(HarnessError::forbidden(
                        "Source folders overlap privileged runtime, account or control paths.",
                    ));
                }
            }
        }
    }
    Ok(spec)
}
pub fn trusted_helper(workspace: &Path) -> Result<PathBuf> {
    let workspace = canonical(workspace)?;
    let control = workspace
        .parent()
        .ok_or_else(|| HarnessError::invalid("Sandbox workspace needs a control directory."))?;
    crate::util::private_directory(control)?;
    let executable = std::env::current_exe()?;
    let parent = executable
        .parent()
        .ok_or_else(|| HarnessError::invalid("Native helper has no parent."))?;
    let name = executable
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| HarnessError::invalid("Native helper filename is invalid."))?;
    let mut source = infra_core::bank::publication::PublicationFiles::new(parent)?;
    let (bytes, _identity) = source
        .capture_source(name, 512 * 1024 * 1024)?
        .ok_or_else(|| HarnessError::unavailable("The native helper is unavailable."))?;
    let digest = store::sha(&bytes);
    let filename = if cfg!(windows) {
        "infra-engine.exe"
    } else {
        "infra-engine"
    };
    let relative = format!("native-helpers/{digest}/{filename}");
    let mut destination = infra_core::bank::publication::PublicationFiles::new(control)?;
    let previous = destination.read(&relative, 512 * 1024 * 1024)?;
    if previous.as_ref().is_some_and(|old| old != &bytes) {
        return Err(HarnessError::conflict("A pinned native helper changed."));
    }
    if previous.is_none() {
        destination.write_expected_mode_checked(
            &relative,
            &bytes,
            None,
            None,
            0o500,
            512 * 1024 * 1024,
        )?;
    } else if destination.read_mode(&relative)? != Some(if cfg!(windows) { 0o444 } else { 0o500 }) {
        destination.set_mode(&relative, 0o500)?;
    }
    source.check()?;
    destination.check()?;
    Ok(control.join(relative))
}
pub fn creator(spec: &Spec, argv: &[String]) -> Result<Vec<String>> {
    command(spec, argv, false)
}
pub fn discovery(spec: &Spec, argv: &[String]) -> Result<Vec<String>> {
    command(spec, argv, true)
}
fn command(spec: &Spec, argv: &[String], discovery: bool) -> Result<Vec<String>> {
    if argv.is_empty() {
        return Err(HarnessError::invalid("Sandbox native command is missing."));
    }
    let spec = prepared(spec, discovery)?;
    let control = spec
        .workspace
        .parent()
        .ok_or_else(|| HarnessError::invalid("Missing sandbox control parent."))?;
    #[cfg(windows)]
    {
        let codex=spec.codex.as_ref().ok_or_else(||HarnessError::unavailable("Windows isolation requires native Codex with configured elevated permission profiles."))?;
        let mut entries = Map::new();
        entries.insert(
            if discovery { ":minimal" } else { ":root" }.into(),
            "read".into(),
        );
        for path in spec
            .runtime
            .iter()
            .chain(spec.readable.iter())
            .map(PathBuf::as_path)
            .chain([control])
        {
            entries.insert(arg(path)?, "read".into());
        }
        for path in &spec.accounts {
            entries.insert(arg(path)?, "write".into());
        }
        entries.insert(arg(&spec.workspace)?, "write".into());
        entries.insert(
            arg(&spec.target)?,
            if discovery { "deny" } else { "read" }.into(),
        );
        for path in &spec.protected {
            entries.insert(arg(path)?, if discovery { "deny" } else { "read" }.into());
        }
        if let Some(evidence) = &spec.evidence {
            entries.insert(arg(evidence)?, "read".into());
        }
        let name = format!(
            "harness{}{}",
            if discovery { "discovery" } else { "creator" },
            store::hexid()
        );
        let mut roots = Map::new();
        roots.insert(arg(&spec.workspace)?, true.into());
        let profile =
            json!({"filesystem":entries,"network":{"enabled":true},"workspace_roots":roots});
        let mut permissions = Map::new();
        permissions.insert(name.clone(), profile);
        let tmp = spec.workspace.join(".harness-tmp");
        infra_core::fs::ensure_directory(&tmp, true)?;
        let mut out = provider::command_argv(&[arg(codex)?], Some("codex"))?;
        out.extend([
            "sandbox".into(),
            "-c".into(),
            "windows.sandbox=\"elevated\"".into(),
            "-c".into(),
            format!("permissions={}", toml(&permissions.into())?),
            "-P".into(),
            name,
            "-C".into(),
            arg(&spec.workspace)?,
            "--include-managed-config".into(),
            "--".into(),
            arg(&trusted_helper(&spec.workspace)?)?,
            "internal-sandbox-child".into(),
            arg(&tmp)?,
        ]);
        out.extend_from_slice(argv);
        Ok(out)
    }
    #[cfg(not(windows))]
    {
        if let Some(bwrap) = provider::find_executable("bwrap") {
            // The private scratch folder exists on every platform: the discovery
            // probe proves the workspace is writable by creating a file in it.
            infra_core::fs::ensure_directory(&spec.workspace.join(".harness-tmp"), true)?;
            let mut out = vec![
                arg(&bwrap)?,
                "--die-with-parent".into(),
                "--new-session".into(),
                "--unshare-pid".into(),
            ];
            let public = [
                "/usr",
                "/bin",
                "/sbin",
                "/lib",
                "/lib64",
                "/etc/ssl/certs",
                "/etc/pki/tls/certs",
                "/etc/ca-certificates",
                "/etc/ld.so.cache",
                "/etc/ld.so.conf",
                "/etc/nsswitch.conf",
                "/etc/passwd",
                "/etc/group",
                "/etc/hosts",
                "/etc/resolv.conf",
            ];
            if discovery {
                for name in public {
                    let path = Path::new(name);
                    if path.exists() {
                        let resolved = infra_core::fs::canonicalize(path)?;
                        if spec.protected.iter().any(|p| overlap(p, &resolved)) {
                            return Err(HarnessError::forbidden(
                                "Source folders must be outside OS runtime directories.",
                            ));
                        }
                        out.extend(["--ro-bind".into(), name.into(), name.into()]);
                    }
                }
                for path in spec.runtime.iter().chain(spec.readable.iter()) {
                    let name = arg(path)?;
                    out.extend(["--ro-bind".into(), name.clone(), name]);
                }
            } else {
                out.extend(["--ro-bind", "/", "/"].map(String::from));
            }
            out.extend(["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"].map(String::from));
            for path in &spec.accounts {
                let name = arg(path)?;
                out.extend(["--bind".into(), name.clone(), name]);
            }
            let control = arg(control)?;
            out.extend(["--ro-bind".into(), control.clone(), control]);
            let workspace = arg(&spec.workspace)?;
            out.extend(["--bind".into(), workspace.clone(), workspace.clone()]);
            if !discovery {
                for path in spec.protected.iter().chain([&spec.target]) {
                    let name = arg(path)?;
                    out.extend(["--ro-bind".into(), name.clone(), name]);
                }
            }
            if let Some(evidence) = &spec.evidence {
                let name = arg(evidence)?;
                out.extend(["--ro-bind".into(), name.clone(), name]);
            }
            out.extend(["--chdir".into(), workspace, "--".into()]);
            out.extend_from_slice(argv);
            return Ok(out);
        }
        #[cfg(target_os = "macos")]
        {
            let sandbox = Path::new("/usr/bin/sandbox-exec");
            if !sandbox.is_file() {
                return Err(HarnessError::unavailable(
                    "macOS isolation requires sandbox-exec.",
                ));
            }
            let mut profile = if discovery {
                "(version 1)\n(allow default)\n(deny file-read*)\n(deny file-write*)\n".to_string()
            } else {
                "(version 1)\n(allow default)\n(deny file-write*)\n".into()
            };
            let public = [
                "/usr",
                "/bin",
                "/sbin",
                "/System",
                "/Library/Apple",
                "/private/etc/ssl",
                "/private/etc/hosts",
                "/private/etc/resolv.conf",
                "/dev",
            ];
            if discovery {
                for name in public {
                    let path = Path::new(name);
                    if path.exists() {
                        if spec.protected.iter().any(|p| overlap(p, path)) {
                            return Err(HarnessError::forbidden(
                                "Source folders must be outside OS runtime directories.",
                            ));
                        }
                        profile
                            .push_str(&format!("(allow file-read* (subpath {}))\n", sbpl(path)?));
                    }
                }
                for path in spec
                    .runtime
                    .iter()
                    .chain(spec.readable.iter())
                    .chain(spec.accounts.iter())
                    .map(PathBuf::as_path)
                    .chain([control])
                {
                    profile.push_str(&format!("(allow file-read* (subpath {}))\n", sbpl(path)?));
                }
            }
            for path in spec.accounts.iter().chain([&spec.workspace]) {
                profile.push_str(&format!("(allow file-write* (subpath {}))\n", sbpl(path)?));
            }
            if !discovery {
                for path in spec.protected.iter().chain([&spec.target]) {
                    profile.push_str(&format!("(deny file-write* (subpath {}))\n", sbpl(path)?));
                }
            }
            if let Some(evidence) = &spec.evidence {
                profile.push_str(&format!(
                    "(deny file-write* (subpath {}))\n",
                    sbpl(evidence)?
                ));
            }
            let path = control.join(format!(
                "sandbox-{}.sb",
                &store::sha(profile.as_bytes())[..16]
            ));
            infra_core::fs::atomic_write(&path, profile.as_bytes())?;
            let tmp = spec.workspace.join(".harness-tmp");
            infra_core::fs::ensure_directory(&tmp, true)?;
            let mut out = vec![
                arg(sandbox)?,
                "-f".into(),
                arg(&path)?,
                "--".into(),
                "/usr/bin/env".into(),
            ];
            for key in ["TMPDIR", "TMP", "TEMP"] {
                out.push(format!("{key}={}", tmp.display()));
            }
            out.extend_from_slice(argv);
            return Ok(out);
        }
        Err(HarnessError::unavailable(
            "AI isolation requires bubblewrap on Linux or sandbox-exec on macOS; unrestricted fallback is refused.",
        ))
    }
}
pub fn child_entry(arguments: &[String]) -> Result<i32> {
    let (tmp, argv) = arguments
        .split_first()
        .ok_or_else(|| HarnessError::invalid("Missing private temporary directory."))?;
    let (executable, args) = argv
        .split_first()
        .ok_or_else(|| HarnessError::invalid("Missing sandbox child command."))?;
    let tmp = canonical(Path::new(tmp))?;
    let mut child = std::process::Command::new(executable);
    child.args(args);
    for key in ["TMPDIR", "TMP", "TEMP"] {
        child.env(key, &tmp);
    }
    Ok(child.status()?.code().unwrap_or(1))
}
fn readable(path: &Path) -> bool {
    std::fs::read_dir(path).is_ok() || std::fs::File::open(path).is_ok()
}
/// Runs inside the discovery sandbox. Every outcome, failure included, is one JSON
/// line on stdout: the caller never sees this process's stderr, so a refusal must
/// name its own reason there.
pub fn probe_entry(arguments: &[String]) -> Result<i32> {
    match probe(arguments) {
        Ok(receipt) => {
            println!("{receipt}");
            Ok(0)
        }
        Err(error) => {
            println!("{}", json!({"ok":false,"error":error.to_string()}));
            Ok(1)
        }
    }
}
fn probe(arguments: &[String]) -> Result<Value> {
    if arguments.len() != 3 {
        return Err(HarnessError::invalid("Missing discovery probe paths."));
    }
    let listing = infra_core::fs::read_text(Path::new(&arguments[0])).map_err(|_| {
        HarnessError::unavailable("The sandbox could not read the list of paths to check.")
    })?;
    if listing.len() > 32 * 1024 * 1024 {
        return Err(HarnessError::invalid(
            "Boundary path listing exceeds its limit.",
        ));
    }
    let mut allowed = false;
    let mut count = 0;
    let mut denied_count = 0;
    let mut allowed_count = 0;
    for line in listing.lines() {
        if line == "--readable" {
            allowed = true;
            continue;
        }
        if line.is_empty() {
            continue;
        }
        count += 1;
        if count > 100_100 {
            return Err(HarnessError::invalid(
                "Boundary path count exceeds its limit.",
            ));
        }
        if allowed {
            allowed_count += 1;
        } else {
            denied_count += 1;
        }
        let can = readable(Path::new(line));
        if !allowed && can {
            return Err(HarnessError::forbidden(format!(
                "The sandbox could read an original file or folder: {line}"
            )));
        }
        if allowed && !can {
            return Err(HarnessError::unavailable(
                "The sandbox could not read captured evidence.",
            ));
        }
    }
    let workspace = Path::new(&arguments[1]);
    let evidence = Path::new(&arguments[2]);
    let scratch = workspace
        .join(".harness-tmp")
        .join(format!("probe-{}", store::hexid()));
    std::fs::write(&scratch, b"workspace probe")
        .and_then(|()| std::fs::remove_file(&scratch))
        .map_err(|error| {
            HarnessError::unavailable(format!(
                "The sandbox workspace is not writable ({}): {error}",
                scratch.display()
            ))
        })?;
    let denied = evidence.join(format!("probe-{}", store::hexid()));
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&denied)
    {
        Ok(mut file) => {
            let _ = file.write_all(b"evidence probe");
            drop(file);
            let _ = std::fs::remove_file(denied);
            Err(HarnessError::forbidden(
                "The sandbox could change captured evidence.",
            ))
        }
        Err(_) => Ok(
            json!({"ok":true,"denied":denied_count,"allowed":allowed_count,"evidence_read_only":true,"workspace_writable":true}),
        ),
    }
}
pub fn original_paths(roots: &[PathBuf]) -> Result<Vec<PathBuf>> {
    fn visit(path: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<()> {
        if depth > 64 || out.len() >= 100_000 {
            return Err(HarnessError::invalid(
                "Discovery boundary enumeration exceeds its limit.",
            ));
        }
        out.push(path.into());
        let meta = std::fs::symlink_metadata(path)?;
        if meta.is_dir() && !infra_core::fs::linked(&meta) {
            let mut entries = std::fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                visit(&entry.path(), depth + 1, out)?;
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    for root in roots {
        if roots
            .iter()
            .any(|other| other != root && root.starts_with(other))
        {
            continue;
        }
        visit(root, 0, &mut out)?;
    }
    Ok(out)
}
pub struct BoundaryRequest {
    pub sentinel: PathBuf,
    pub listing: PathBuf,
    pub command: Vec<String>,
    pub denied: usize,
    pub allowed: usize,
}
impl Drop for BoundaryRequest {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.listing);
        let _ = std::fs::remove_file(&self.sentinel);
    }
}
pub fn boundary_request(spec: &Spec) -> Result<BoundaryRequest> {
    let workspace = canonical(&spec.workspace)?;
    let evidence = spec
        .evidence
        .as_ref()
        .ok_or_else(|| HarnessError::invalid("Discovery evidence directory is missing."))?;
    let control = workspace
        .parent()
        .ok_or_else(|| HarnessError::invalid("Discovery control directory is missing."))?;
    let mut lines = original_paths(&spec.protected)?
        .into_iter()
        .map(|p| arg(&p))
        .collect::<Result<Vec<_>>>()?;
    let denied = lines.len() + 1;
    let sentinel =
        std::env::temp_dir().join(format!("harness-discovery-sentinel-{}", store::hexid()));
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&sentinel)?;
    crate::util::private_file(&sentinel)?;
    lines.push(arg(&sentinel)?);
    lines.push("--readable".into());
    let mut allowed = 0;
    for entry in std::fs::read_dir(evidence)?.take(3) {
        lines.push(arg(&entry?.path())?);
        allowed += 1;
    }
    let listing = control.join(format!("boundary-paths-{}.txt", store::hexid()));
    infra_core::fs::atomic_write(&listing, format!("{}\n", lines.join("\n")).as_bytes())?;
    let mut receipt = BoundaryRequest {
        sentinel,
        listing,
        command: vec![],
        denied,
        allowed,
    };
    let argv = vec![
        arg(&trusted_helper(&workspace)?)?,
        "internal-discovery-probe".into(),
        arg(&receipt.listing)?,
        arg(&workspace)?,
        arg(evidence)?,
    ];
    receipt.command = discovery(spec, &argv)?;
    Ok(receipt)
}
