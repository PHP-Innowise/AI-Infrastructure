//! Executable support for native sandbox acceptance. This binary is not an
//! installed runtime artifact; Cargo builds it for the ignored integration test.
#[cfg(not(any(windows, target_os = "linux")))]
fn main() {
    eprintln!("native Windows sandbox fixture requires Windows");
    std::process::exit(2);
}
#[cfg(target_os = "linux")]
#[path = "process_fixture_linux.rs"]
mod linux;
#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = linux::entry() {
        eprintln!("native Linux process fixture failed: {error}");
        std::process::exit(1);
    }
}
#[cfg(windows)]
fn main() {
    if let Err(error) = native::entry() {
        eprintln!("native sandbox fixture failed: {error}");
        std::process::exit(1);
    }
}
#[cfg(windows)]
mod native {
    use infra_host::{
        HostError, Result,
        process::{self, Limits},
        provider,
        sandbox::{self, Spec},
    };
    use serde_json::{Value, json};
    use std::{
        collections::BTreeMap,
        fs::{self, File, OpenOptions},
        io::{Read, Write},
        path::{Path, PathBuf},
        process::{Command, Stdio},
        sync::{Arc, atomic::AtomicBool},
        time::Duration,
    };
    const BINARY_INPUT: &[u8] = b"native input\r\nsecond line\n\0\xff";
    fn fail(message: impl Into<String>) -> HostError {
        HostError::unavailable(message)
    }
    fn require(condition: bool, message: &str) -> Result<()> {
        if condition {
            Ok(())
        } else {
            Err(fail(message))
        }
    }
    fn text(path: &Path) -> Result<String> {
        path.to_str()
            .map(str::to_owned)
            .ok_or_else(|| fail("fixture path must be UTF-8"))
    }
    fn argument(args: &[String], at: usize) -> Result<PathBuf> {
        args.get(at)
            .map(PathBuf::from)
            .ok_or_else(|| fail("fixture argument missing"))
    }
    fn receipt(value: Value) -> Result<()> {
        println!("{}", serde_json::to_string(&value)?);
        Ok(())
    }
    fn deny_write(path: &Path) -> Result<()> {
        require(
            path.parent().is_some_and(Path::is_dir),
            "denied destination parent must exist",
        )?;
        match OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
        {
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Ok(()),
            Err(error) => Err(fail(format!(
                "write failed for an unrelated reason: {error}"
            ))),
            Ok(mut file) => {
                let _ = file.write_all(b"unconfined write");
                Err(fail("sandbox allowed a protected write"))
            }
        }
    }
    fn deny_read(path: &Path) -> Result<()> {
        match fs::read(path) {
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::NotFound
                ) =>
            {
                Ok(())
            }
            Err(error) => Err(fail(format!(
                "read failed for an unrelated reason: {error}"
            ))),
            Ok(_) => Err(fail("discovery could read an original file")),
        }
    }
    fn child_argv(action: &str, paths: &[&Path]) -> Result<Vec<String>> {
        let mut argv = vec![
            text(&std::env::current_exe()?)?,
            "fixture-leaf".into(),
            action.into(),
        ];
        for path in paths {
            argv.push(text(path)?);
        }
        Ok(argv)
    }
    struct Layout {
        root: PathBuf,
        workspace: PathBuf,
        target: PathBuf,
        source: PathBuf,
        account: PathBuf,
        evidence: PathBuf,
        codex: PathBuf,
    }
    impl Layout {
        fn new(root: PathBuf, codex: PathBuf) -> Result<Self> {
            let workspace = root.join("control/agent");
            let target = root.join("project");
            let source = root.join("services/orders");
            let account = root.join("account");
            let evidence = workspace.join("evidence");
            for path in [&root, &workspace, &target, &source, &account, &evidence] {
                infra_host::util::private_directory(path)?;
            }
            fs::create_dir_all(source.join("src"))?;
            fs::create_dir_all(target.join("existing/child"))?;
            fs::write(target.join("composer.json"), b"{}")?;
            fs::write(
                target.join("existing/child/sentinel"),
                b"original descendant",
            )?;
            fs::write(source.join("src/order.rs"), b"ORIGINAL SOURCE")?;
            fs::write(source.join(".env"), b"SECRET=1")?;
            fs::write(root.join("control/private-control"), b"private control")?;
            fs::write(evidence.join("000000.txt"), b"safe evidence")?;
            Ok(Self {
                root,
                workspace,
                target,
                source,
                account,
                evidence,
                codex,
            })
        }
        fn spec(&self, discovery: bool, extra: Vec<PathBuf>) -> Spec {
            let mut protected = vec![self.source.clone()];
            protected.extend(extra);
            Spec {
                workspace: self.workspace.clone(),
                target: self.target.clone(),
                protected,
                readable: vec![],
                accounts: if discovery {
                    vec![]
                } else {
                    vec![self.account.clone()]
                },
                runtime: vec![],
                codex: Some(self.codex.clone()),
                evidence: if discovery {
                    Some(self.evidence.clone())
                } else {
                    None
                },
            }
        }
        fn helper(&self) -> Result<PathBuf> {
            sandbox::trusted_helper(&self.workspace)
        }
        fn assert_originals(&self) -> Result<()> {
            require(
                fs::read(self.source.join("src/order.rs"))? == b"ORIGINAL SOURCE",
                "original source changed",
            )?;
            require(
                fs::read(self.source.join(".env"))? == b"SECRET=1",
                "original private input changed",
            )?;
            require(
                fs::read(self.evidence.join("000000.txt"))? == b"safe evidence",
                "captured evidence changed",
            )?;
            require(
                fs::read(self.root.join("control/private-control"))? == b"private control",
                "control file changed",
            )
        }
    }
    async fn execute(
        argv: &[String],
        cwd: &Path,
        input: &[u8],
        lock: Option<Arc<File>>,
    ) -> Result<process::Output> {
        process::run_with_lock(
            argv,
            cwd,
            &BTreeMap::new(),
            input,
            Limits {
                seconds: Some(180),
                output: 2 * 1024 * 1024,
                line: 1024 * 1024,
            },
            Arc::new(AtomicBool::new(false)),
            lock,
            |_| Ok(()),
        )
        .await
    }
    fn completed(output: &process::Output, label: &str) -> Result<()> {
        if output.code == 0 {
            Ok(())
        } else {
            Err(fail(format!(
                "{label} exited {}; bounded stdout: {}",
                output.code,
                String::from_utf8_lossy(&output.bytes)
                    .chars()
                    .take(2000)
                    .collect::<String>()
            )))
        }
    }
    fn git(root: &Path, argv: &[String]) -> Result<()> {
        let executable = provider::find_executable("git")
            .ok_or_else(|| fail("Git is required for linked-worktree acceptance"))?;
        let global = root.join("empty-gitconfig");
        if !global.exists() {
            fs::write(&global, b"")?;
        }
        let mut command = Command::new(executable);
        command.args([
            "-c",
            "user.name=Native sandbox fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ]);
        command
            .args(argv)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", global)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let output = command.output()?;
        require(
            output.status.success(),
            &format!(
                "native Git fixture failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        )
    }
    async fn creator(mut layout: Layout) -> Result<()> {
        git(&layout.root, &["init".into(), text(&layout.target)?])?;
        git(
            &layout.root,
            &[
                "-C".into(),
                text(&layout.target)?,
                "add".into(),
                "composer.json".into(),
                "existing".into(),
            ],
        )?;
        git(
            &layout.root,
            &[
                "-C".into(),
                text(&layout.target)?,
                "commit".into(),
                "-m".into(),
                "native sandbox fixture".into(),
            ],
        )?;
        let repository = layout.target.clone();
        let linked = layout.root.join("linked project");
        git(
            &layout.root,
            &[
                "-C".into(),
                text(&repository)?,
                "worktree".into(),
                "add".into(),
                "--detach".into(),
                text(&linked)?,
            ],
        )?;
        layout.target = linked;
        let common = repository.join(".git");
        let config_before = fs::read(common.join("config"))?;
        let spec = layout.spec(false, vec![repository.clone(), common.clone()]);
        let helper = layout.helper()?;
        let control = layout.workspace.parent().unwrap();
        let leaf = vec![
            text(&helper)?,
            "fixture-leaf".into(),
            "creator".into(),
            text(&layout.workspace)?,
            text(&layout.target)?,
            text(control)?,
            text(&layout.source)?,
            text(&layout.account)?,
            text(&common)?,
        ];
        let argv = sandbox::creator(&spec, &leaf)?;
        let again = sandbox::creator(&spec, &leaf)?;
        let profile = |command: &[String]| {
            command
                .iter()
                .position(|s| s == "-P")
                .and_then(|at| command.get(at + 1))
                .cloned()
        };
        require(
            profile(&argv).is_some() && profile(&argv) != profile(&again),
            "permission profiles must have distinct names",
        )?;
        require(
            argv.iter().any(|word| word == "--include-managed-config"),
            "managed sandbox configuration is required",
        )?;
        require(
            !argv.iter().any(|word| word.contains("danger-full-access")),
            "creator boundary must not disable sandbox enforcement",
        )?;
        let output = execute(&argv, &layout.workspace, BINARY_INPUT, None).await?;
        completed(&output, "creator boundary")?;
        require(
            output
                .bytes
                .windows(b"creator-boundary-ok".len())
                .any(|window| window == b"creator-boundary-ok"),
            "creator leaf did not complete all assertions",
        )?;
        let echo = sandbox::creator(
            &spec,
            &[text(&helper)?, "fixture-leaf".into(), "echo".into()],
        )?;
        let output = execute(&echo, &layout.workspace, BINARY_INPUT, None).await?;
        completed(&output, "binary stdin/stdout")?;
        require(
            output.bytes == BINARY_INPUT,
            "sandbox trampoline changed binary stdin or stdout",
        )?;
        require(
            fs::read(layout.workspace.join("allowed"))? == b"workspace write",
            "workspace write was not retained",
        )?;
        require(
            fs::read(layout.account.join("allowed-account"))? == b"account write",
            "explicit account write was not retained",
        )?;
        require(
            fs::read(common.join("config"))? == config_before,
            "Git common-directory configuration changed",
        )?;
        require(
            fs::read(layout.target.join("existing/child/sentinel"))? == b"original descendant",
            "linked-worktree descendant changed",
        )?;
        require(
            fs::read(repository.join("existing/child/sentinel"))? == b"original descendant",
            "original checkout descendant changed",
        )?;
        for path in [
            layout.target.join("denied"),
            layout.target.join("child-denied"),
            layout.target.join("renamed"),
            control.join("denied"),
            layout.source.join("denied"),
        ] {
            require(!path.exists(), "protected destination appeared")?;
        }
        layout.assert_originals()?;
        receipt(
            json!({"creator_boundary":true,"binary_stdio":true,"descendant_denied":true,
        "originals_unchanged":true,"private_temp":true,"random_profile":true}),
        )
    }
    fn probe_receipt(output: &process::Output, request: &sandbox::BoundaryRequest) -> Result<()> {
        completed(output, "discovery boundary probe")?;
        let receipts = output
            .bytes
            .split(|b| *b == b'\n')
            .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
            .filter(|value| value.get("ok").is_some())
            .collect::<Vec<_>>();
        require(
            receipts.len() == 1,
            "discovery boundary must return one unambiguous receipt",
        )?;
        let receipt = &receipts[0];
        require(
            receipt["ok"] == true
                && receipt["denied"].as_u64() == Some(request.denied as u64)
                && receipt["allowed"].as_u64() == Some(request.allowed as u64)
                && receipt["evidence_read_only"] == true
                && receipt["workspace_writable"] == true,
            "discovery probe receipt does not attest the requested boundary",
        )
    }
    async fn discovery(layout: Layout, explicit_acl: bool) -> Result<()> {
        let exposed = layout.source.join("shared.txt");
        if explicit_acl {
            fs::write(&exposed, b"EXPLICITLY SHARED")?;
            let system = std::env::var_os("SystemRoot")
                .map(PathBuf::from)
                .ok_or_else(|| fail("SystemRoot missing"))?
                .join("System32/icacls.exe");
            let output = Command::new(system)
                .arg(&exposed)
                .args(["/grant", "*S-1-1-0:(R)"])
                .stdin(Stdio::null())
                .output()?;
            require(
                output.status.success(),
                "could not create the explicit Everyone-read ACL fixture",
            )?;
        }
        let spec = layout.spec(true, vec![]);
        let boundary = sandbox::boundary_request(&spec)?;
        require(
            boundary.allowed >= 1 && boundary.denied >= 4,
            "boundary fixture must enumerate evidence and original paths",
        )?;
        let output = process::run_raw_with_lock(
            &boundary.command,
            &layout.workspace,
            &BTreeMap::new(),
            b"",
            Limits {
                seconds: Some(180),
                output: 2 * 1024 * 1024,
                line: 1024 * 1024,
            },
            Arc::new(AtomicBool::new(false)),
            None,
            |_| Ok(()),
        )
        .await?;
        if output.code != 0 {
            let diagnostic = String::from_utf8_lossy(&output.bytes);
            // Only the deliberate explicit-ACL exposure can be a valid refusal.
            // Missing capabilities, runtime launch failures and unrelated errors fail.
            require(
                explicit_acl
                    && diagnostic.contains("The sandbox could read an original file or folder:"),
                "discovery preflight failed without identifying readable original data",
            )?;
            require(
                !layout.workspace.join("scan-started").exists(),
                "scan started after failed boundary preflight",
            )?;
            layout.assert_originals()?;
            require(
                fs::read(&exposed)? == b"EXPLICITLY SHARED",
                "explicitly shared source changed",
            )?;
            return receipt(
                json!({"outcome":"preflight-refused","scan_started":false,"originals_unchanged":true,"evidence_read_only":true}),
            );
        }
        probe_receipt(&output, &boundary)?;
        let helper = layout.helper()?;
        let argv = sandbox::discovery(
            &spec,
            &[
                text(&helper)?,
                "fixture-leaf".into(),
                "discovery".into(),
                text(&layout.workspace)?,
                text(&layout.source)?,
                text(&layout.evidence)?,
            ],
        )?;
        let output = execute(&argv, &layout.workspace, b"", None).await?;
        completed(&output, "discovery scan")?;
        require(
            output
                .bytes
                .windows(b"discovery-boundary-ok".len())
                .any(|window| window == b"discovery-boundary-ok"),
            "discovery leaf did not complete all assertions",
        )?;
        require(
            fs::read(layout.workspace.join("scan-started"))? == b"allowed",
            "scan did not execute",
        )?;
        layout.assert_originals()?;
        if explicit_acl {
            require(
                fs::read(&exposed)? == b"EXPLICITLY SHARED",
                "explicitly shared source changed",
            )?;
        }
        let denied = boundary.denied;
        let listing = boundary.listing.clone();
        let sentinel = boundary.sentinel.clone();
        drop(boundary);
        require(
            !listing.exists() && !sentinel.exists(),
            "boundary request did not remove private probe files",
        )?;
        receipt(if explicit_acl {
            json!({"outcome":"scan-denied-original","scan_completed":true,"originals_unchanged":true,"evidence_read_only":true})
        } else {
            json!({"boundary_verified":true,"scan_completed":true,"denied_paths":denied,"originals_unchanged":true,"evidence_read_only":true})
        })
    }
    async fn owner(layout: Layout) -> Result<()> {
        use std::os::windows::fs::OpenOptionsExt;
        let lock = Arc::new(
            OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .share_mode(0)
                .open(layout.root.join("runner.lock"))?,
        );
        let helper = layout.helper()?;
        let marker = layout.workspace.join("pids.json");
        let argv = sandbox::creator(
            &layout.spec(false, vec![]),
            &[
                text(&helper)?,
                "fixture-leaf".into(),
                "linger".into(),
                text(&marker)?,
            ],
        )?;
        let output = process::run_with_lock(
            &argv,
            &layout.workspace,
            &BTreeMap::new(),
            b"",
            Limits {
                seconds: Some(300),
                output: 128 * 1024,
                line: 65536,
            },
            Arc::new(AtomicBool::new(false)),
            Some(lock),
            |_| Ok(()),
        )
        .await?;
        Err(fail(format!(
            "owner scenario ended before the integration test terminated its owner: {}",
            output.code
        )))
    }
    fn leaf(args: &[String]) -> Result<i32> {
        let action = args
            .first()
            .ok_or_else(|| fail("fixture leaf action missing"))?;
        match action.as_str() {
            "echo" => {
                let mut input = Vec::new();
                std::io::stdin().take(65536).read_to_end(&mut input)?;
                std::io::stdout().write_all(&input)?;
                std::io::stdout().flush()?;
            }
            "deny-write" => {
                deny_write(&argument(args, 1)?)?;
            }
            "sleep" => {
                std::thread::sleep(Duration::from_secs(300));
            }
            "linger" => {
                let marker = argument(args, 1)?;
                let argv = child_argv("sleep", &[])?;
                let child = Command::new(&argv[0])
                    .args(&argv[1..])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()?;
                fs::write(
                    marker,
                    serde_json::to_vec(&json!([std::process::id(), child.id()]))?,
                )?;
                std::thread::sleep(Duration::from_secs(300));
            }
            "creator" => {
                let workspace = argument(args, 1)?;
                let target = argument(args, 2)?;
                let control = argument(args, 3)?;
                let source = argument(args, 4)?;
                let account = argument(args, 5)?;
                let common = argument(args, 6)?;
                let mut input = Vec::new();
                std::io::stdin().take(65536).read_to_end(&mut input)?;
                require(input == BINARY_INPUT, "binary input was changed")?;
                for key in ["TEMP", "TMP"] {
                    let actual = std::env::var_os(key)
                        .map(PathBuf::from)
                        .ok_or_else(|| fail("sandbox child temporary variable missing"))?;
                    require(
                        fs::canonicalize(actual)?
                            == fs::canonicalize(workspace.join(".harness-tmp"))?,
                        "sandbox child did not set its private temporary directory",
                    )?;
                }
                require(
                    fs::read(target.join("composer.json"))? == b"{}",
                    "creator cannot read target sources",
                )?;
                require(
                    fs::read(target.join("existing/child/sentinel"))? == b"original descendant",
                    "creator cannot read existing descendants",
                )?;
                require(
                    !fs::read(common.join("config"))?.is_empty(),
                    "creator cannot read Git common directory",
                )?;
                fs::write(workspace.join("allowed"), b"workspace write")?;
                fs::write(account.join("allowed-account"), b"account write")?;
                for path in [
                    target.join("denied"),
                    target.join("existing/child/sentinel"),
                    common.join("config"),
                    control.join("denied"),
                    source.join("denied"),
                ] {
                    deny_write(&path)?;
                }
                let argv = child_argv("deny-write", &[&target.join("child-denied")])?;
                let child = Command::new(&argv[0])
                    .args(&argv[1..])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()?;
                require(
                    child.success(),
                    "protected-write denial failed in a native descendant",
                )?;
                match fs::rename(workspace.join("allowed"), target.join("renamed")) {
                    Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {}
                    Err(error) => {
                        return Err(fail(format!(
                            "rename failed for an unrelated reason: {error}"
                        )));
                    }
                    Ok(()) => return Err(fail("sandbox allowed rename into target")),
                };
                println!("creator-boundary-ok");
            }
            "discovery" => {
                let workspace = argument(args, 1)?;
                let source = argument(args, 2)?;
                let evidence = argument(args, 3)?;
                for path in [source.join("src/order.rs"), source.join(".env")] {
                    deny_read(&path)?;
                }
                let shared = source.join("shared.txt");
                deny_read(&shared)?;
                require(
                    fs::read(evidence.join("000000.txt"))? == b"safe evidence",
                    "discovery cannot read captured evidence",
                )?;
                deny_write(&evidence.join("000000.txt"))?;
                deny_write(&evidence.join("new-evidence"))?;
                fs::write(workspace.join("scan-started"), b"allowed")?;
                println!("discovery-boundary-ok");
            }
            _ => return Err(fail("unknown fixture leaf action")),
        }
        Ok(0)
    }
    pub fn entry() -> Result<()> {
        let args = std::env::args().skip(1).collect::<Vec<_>>();
        let (action, rest) = args
            .split_first()
            .ok_or_else(|| fail("fixture action missing"))?;
        let code = match action.as_str() {
            "internal-guard" => Some(process::guard_entry(rest)?),
            "internal-sandbox-child" => Some(sandbox::child_entry(rest)?),
            "internal-discovery-probe" => Some(sandbox::probe_entry(rest)?),
            "fixture-leaf" => Some(leaf(rest)?),
            _ => None,
        };
        if let Some(code) = code {
            std::process::exit(code);
        }
        require(
            std::env::var("HARNESS_WINDOWS_CREATOR_SANDBOX").as_deref() == Ok("1"),
            "explicit native Windows sandbox environment flag is required",
        )?;
        let root = argument(rest, 0)?;
        let codex = argument(rest, 1)?;
        require(codex.is_file(), "configured native Codex is unavailable")?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        let layout = Layout::new(root, codex)?;
        runtime.block_on(async {
            match action.as_str() {
                "creator" => creator(layout).await,
                "discovery" => discovery(layout, false).await,
                "discovery-explicit-acl" => discovery(layout, true).await,
                "owner" => owner(layout).await,
                _ => Err(fail("unknown native acceptance scenario")),
            }
        })
    }
}
