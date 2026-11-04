use crate::{
    Result, artifact,
    process::{self, CommandSpec},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};
pub const BACKEND: &str = "linux-bwrap-allowlist-v1";
const PUBLIC: [&str; 14] = [
    "/usr",
    "/bin",
    "/sbin",
    "/lib",
    "/lib64",
    "/etc/ssl/certs",
    "/etc/ssl/cert.pem",
    "/etc/pki/tls/certs",
    "/etc/ld.so.cache",
    "/etc/nsswitch.conf",
    "/etc/passwd",
    "/etc/group",
    "/etc/hosts",
    "/etc/resolv.conf",
];
pub fn which(name: &str) -> Option<PathBuf> {
    let path = Path::new(name);
    if path.components().count() > 1 {
        return path.is_file().then(|| path.to_path_buf());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|p| p.join(name))
        .find(|p| p.is_file())
}
#[derive(Clone, Debug)]
pub struct View {
    pub prefix: Vec<String>,
    pub command: Vec<String>,
    pub corpus: PathBuf,
}
impl View {
    pub fn spec(&self) -> CommandSpec {
        let mut argv = self.prefix.clone();
        argv.push("--".into());
        argv.extend(self.command.clone());
        CommandSpec {
            argv,
            cwd: self.corpus.clone(),
            env: BTreeMap::new(),
            input: vec![],
        }
    }
}
pub fn prepare(
    corpus: &Path,
    private: &Path,
    command: &[String],
    credentials: bool,
    citations: Option<&Path>,
) -> Result<View> {
    if !cfg!(target_os = "linux") {
        return Err(artifact::invalid(
            "task-complete requires Linux bubblewrap read isolation",
        ));
    }
    let bwrap = which("bwrap").ok_or_else(|| {
        artifact::invalid("task-complete requires Linux bubblewrap read isolation")
    })?;
    let corpus = infra_core::fs::canonicalize(artifact::ordinary(corpus)?)?;
    let private = artifact::ordinary(private)?;
    if !corpus.is_dir() || artifact::overlaps(&corpus, &private)? {
        return Err(artifact::invalid("private inputs overlap agent corpus"));
    }
    let mut roots = PUBLIC
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect::<Vec<_>>();
    let mut native = command.to_vec();
    if let Some(program) = native.first_mut() {
        let resolved = infra_core::fs::canonicalize(
            which(program).ok_or_else(|| artifact::invalid("native executable unavailable"))?,
        )?;
        let package = resolved
            .ancestors()
            .find(|p| p.file_name().is_some_and(|n| n == "node_modules"))
            .unwrap_or_else(|| resolved.parent().unwrap_or(&resolved))
            .to_path_buf();
        if !roots.iter().any(|r| resolved.starts_with(r)) {
            roots.push(package);
        }
        *program = resolved.to_string_lossy().into();
    }
    for root in &roots {
        if artifact::overlaps(&private, root)? || artifact::overlaps(&corpus, root)? {
            return Err(artifact::invalid(
                "private inputs or corpus overlap allowed runtime roots",
            ));
        }
    }
    let mut prefix = vec![
        bwrap.to_string_lossy().into(),
        "--die-with-parent".into(),
        "--new-session".into(),
        "--unshare-pid".into(),
        "--tmpfs".into(),
        "/".into(),
        "--proc".into(),
        "/proc".into(),
        "--dev".into(),
        "/dev".into(),
        "--tmpfs".into(),
        "/tmp".into(),
        "--dir".into(),
        "/home".into(),
        "--dir".into(),
        "/home/agent".into(),
        "--dir".into(),
        "/home/agent/.codex".into(),
    ];
    for root in roots {
        prefix.extend([
            "--ro-bind".into(),
            root.to_string_lossy().into(),
            root.to_string_lossy().into(),
        ]);
    }
    prefix.extend([
        "--bind".into(),
        corpus.to_string_lossy().into(),
        corpus.to_string_lossy().into(),
        "--chdir".into(),
        corpus.to_string_lossy().into(),
        "--clearenv".into(),
        "--setenv".into(),
        "HOME".into(),
        "/home/agent".into(),
        "--setenv".into(),
        "CODEX_HOME".into(),
        "/home/agent/.codex".into(),
        "--setenv".into(),
        "PATH".into(),
        "/usr/local/bin:/usr/bin:/bin".into(),
        "--setenv".into(),
        "LANG".into(),
        "C.UTF-8".into(),
    ]);
    if let Some(root) = citations {
        let root = artifact::ordinary(root)?;
        if !root.is_dir()
            || artifact::overlaps(&root, &corpus)?
            || artifact::overlaps(&root, &private)?
        {
            return Err(artifact::invalid(
                "citation snapshot overlaps private or mutable roots",
            ));
        }
        let mount = corpus.join(crate::intake::MOUNT_NAME);
        if mount.exists() {
            return Err(artifact::invalid("reserved citation mount exists"));
        }
        artifact::private_dir(&mount)?;
        prefix.extend([
            "--ro-bind".into(),
            root.to_string_lossy().into(),
            mount.to_string_lossy().into(),
        ]);
    }
    if credentials {
        let home = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".codex")
            });
        let auth = home.join("auth.json");
        if auth.is_file() {
            if artifact::overlaps(&private, &auth)? {
                return Err(artifact::invalid(
                    "private inputs overlap native account state",
                ));
            }
            prefix.extend([
                "--ro-bind".into(),
                infra_core::fs::canonicalize(auth)?.to_string_lossy().into(),
                "/home/agent/.codex/auth.json".into(),
            ]);
        }
    }
    Ok(View {
        prefix,
        command: native,
        corpus,
    })
}
pub fn probe(view: &View, sentinel: &Path) -> bool {
    let mut probe = view.clone();
    probe.command = vec![
        "/usr/bin/test".into(),
        "!".into(),
        "-e".into(),
        sentinel.to_string_lossy().into(),
    ];
    process::capture(&probe.spec(), Duration::from_secs(10), 4096)
        .is_ok_and(|(code, raw)| code == 0 && raw.is_empty())
}
pub fn verifier(candidate: &Path, argv: &[String], roots: &[PathBuf]) -> Result<CommandSpec> {
    let sentinel = candidate
        .parent()
        .unwrap_or(candidate)
        .join(".not-an-agent-input");
    let mut view = prepare(candidate, &sentinel, argv, false, None)?;
    for root in roots {
        let root = infra_core::fs::canonicalize(artifact::ordinary(root)?)?;
        if artifact::overlaps(&root, candidate)? {
            return Err(artifact::invalid("verifier root overlaps candidate"));
        }
        view.prefix.extend([
            "--ro-bind".into(),
            root.to_string_lossy().into(),
            root.to_string_lossy().into(),
        ]);
    }
    view.prefix.push("--unshare-net".into());
    Ok(view.spec())
}
