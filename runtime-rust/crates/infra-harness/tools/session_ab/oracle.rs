//! Compile and execute in separate networkless Linux namespaces. Compare all
//! output on the host; exit(0) alone is never an oracle receipt.
use super::{corpus, digest, executable_bytes, ordinary, require, write_json};
use infra_harness::{HarnessError, Result};
use serde_json::{Value as V, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

async fn capture(argv: Vec<String>, cwd: &Path) -> Result<infra_host::process::Output> {
    infra_host::process::run(
        &argv,
        cwd,
        &BTreeMap::new(),
        &[],
        infra_host::process::Limits {
            seconds: Some(20),
            output: 512 * 1024,
            line: 512 * 1024,
        },
        Arc::new(AtomicBool::new(false)),
        |_| Ok(()),
    )
    .await
}
fn receipt(output: infra_host::process::Output) -> Result<(i64, String)> {
    require(output.code == 0, "Oracle sandbox/helper failed.")?;
    // 64 KiB child bytes can expand sixfold when quoted as JSON (e.g. NUL).
    let row = infra_harness::strict_json::parse(&output.bytes, 512 * 1024)?;
    require(
        row["schema"] == "ab-oracle-child-v1",
        "Oracle helper receipt unavailable.",
    )?;
    if row["completed"] != true {
        if matches!(
            row["error_code"].as_str(),
            Some("native-timeout" | "output-limit" | "output-line-limit")
        ) {
            return Err(HarnessError {
                status: 422,
                code: "oracle-candidate-limit",
                message: "Candidate exceeded the verifier execution/output limit.".into(),
            });
        }
        return Err(HarnessError::unavailable(
            "Oracle could not execute or prove cleanup of its child.",
        ));
    }
    let code = row["code"]
        .as_i64()
        .ok_or_else(|| HarnessError::invalid("Oracle exit status missing."))?;
    let text = row["output"]
        .as_str()
        .ok_or_else(|| HarnessError::invalid("Oracle output missing."))?;
    Ok((code, text.to_owned()))
}
pub async fn tools() -> Result<V> {
    require(
        cfg!(target_os = "linux"),
        "This offline code oracle currently requires Linux bubblewrap.",
    )?;
    let cwd = std::env::current_dir()?;
    let root = capture(
        vec!["rustc".into(), "--print".into(), "sysroot".into()],
        &cwd,
    )
    .await?;
    require(root.code == 0, "Rust toolchain unavailable.")?;
    let sysroot = infra_core::fs::canonicalize(String::from_utf8_lossy(&root.bytes).trim())?;
    let compiler = sysroot.join("bin/rustc");
    let version = capture(vec![compiler.to_string_lossy().into(), "-vV".into()], &cwd).await?;
    require(version.code == 0, "Rust compiler identity unavailable.")?;
    let helper = std::env::current_exe()?
        .parent()
        .ok_or_else(|| HarnessError::invalid("Runner directory missing."))?
        .join("infra-harness-ab-oracle");
    let helper = infra_core::fs::canonicalize(helper)?;
    let runtime_roots = ["/usr", "/bin", "/lib", "/lib64"]
        .into_iter()
        .filter(|name| Path::new(name).is_dir())
        .collect::<Vec<_>>();
    let linker = linker(&runtime_roots)?;
    Ok(
        json!({"oracle_helper":helper,"oracle_helper_sha256":digest(&executable_bytes(&helper)?),"sysroot":sysroot,"compiler":compiler,"compiler_sha256":digest(&executable_bytes(&compiler)?),"version":String::from_utf8_lossy(&version.bytes),"linker":linker,"linker_sha256":digest(&executable_bytes(&linker)?),"sandbox":"/usr/bin/bwrap","sandbox_sha256":digest(&executable_bytes(Path::new("/usr/bin/bwrap"))?),"runtime_roots":runtime_roots,"runtime_contract":"compiler executable/version and resolved C linker pinned; actual system runtime directories and sysroot read-only, libraries not content-sealed"}),
    )
}
/// rustc links Linux targets through `cc` on PATH, which is `/usr/bin` in the
/// sandbox. Debian-family hosts resolve that name through `/etc/alternatives`,
/// which the sandbox does not mount, so the oracle passes the host's resolved
/// executable explicitly. It must lie inside a mounted runtime directory.
fn linker(runtime_roots: &[&str]) -> Result<PathBuf> {
    let linker = infra_core::fs::canonicalize("/usr/bin/cc").map_err(|_| {
        HarnessError::unavailable("The default C linker /usr/bin/cc is unavailable.")
    })?;
    require(
        runtime_roots.iter().any(|root| linker.starts_with(root)),
        "The resolved C linker is outside the read-only runtime directories.",
    )?;
    Ok(linker)
}
fn sandbox(input: &Path, work: &Path, tools: &V, program: Vec<String>) -> Vec<String> {
    let mut argv = vec![tools["sandbox"].as_str().unwrap().into()];
    argv.extend(
        [
            "--unshare-all",
            "--die-with-parent",
            "--new-session",
            "--clearenv",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--tmpfs",
            "/tmp",
            "--setenv",
            "PATH",
            "/usr/bin",
        ]
        .map(String::from),
    );
    // Bind the host's actual runtime tree at each standard path. The dynamic
    // loader layout differs between Arch and Ubuntu; do not invent symlinks.
    for root in tools["runtime_roots"].as_array().unwrap() {
        let root = root.as_str().unwrap();
        argv.extend(["--ro-bind".into(), root.into(), root.into()]);
    }
    argv.push("--ro-bind".into());
    let sysroot = tools["sysroot"].as_str().unwrap();
    argv.extend([
        sysroot.into(),
        sysroot.into(),
        "--ro-bind".into(),
        input.to_string_lossy().into(),
        "/input".into(),
        "--bind".into(),
        work.to_string_lossy().into(),
        "/work".into(),
        "--ro-bind".into(),
        tools["oracle_helper"].as_str().unwrap().into(),
        "/helper".into(),
        "--chdir".into(),
        "/work".into(),
        "--".into(),
    ]);
    argv.extend(program);
    argv
}
pub async fn verify(project: &Path, out: &Path, kind: &str, tools: &V) -> V {
    let start = Instant::now();
    let mut result = json!({"correct":null,"status":"unknown","reason":null,"artifact_sha256":null,"oracle_sha256":digest(if kind=="a"{corpus::A_ORACLE.as_bytes()}else{corpus::B_ORACLE.as_bytes()}),"seconds":null});
    let evaluated: Result<bool> = async {
        fs::create_dir(out)?;
        let input = out.join("input");
        let build = out.join("build");
        let runtime = out.join("runtime");
        for dir in [&input, &build, &runtime] {
            fs::create_dir(dir)?;
        }
        let meta = fs::symlink_metadata(project)?;
        require(
            meta.is_dir() && !infra_core::fs::linked(&meta),
            "Unsafe project root.",
        )?;
        for entry in fs::read_dir(project)? {
            let entry = entry?;
            let name = entry.file_name();
            if name == ".git" {
                let meta = entry.metadata()?;
                require(
                    meta.is_dir() && !infra_core::fs::linked(&fs::symlink_metadata(entry.path())?),
                    "Unsafe Git metadata.",
                )?;
            } else if name != "source.rs" && name != "README.md" {
                result["reason"] = "unexpected project file".into();
                return Ok(false);
            }
        }
        for (name, cap) in [("source.rs", 64 * 1024), ("README.md", 4096)] {
            let meta = match fs::symlink_metadata(project.join(name)) {
                Ok(meta) => meta,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    result["reason"] = format!("missing {name}").into();
                    return Ok(false);
                }
                Err(error) => return Err(error.into()),
            };
            let mut invalid = !meta.is_file() || infra_core::fs::linked(&meta) || meta.len() > cap;
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                invalid |= meta.nlink() != 1;
            }
            if invalid {
                result["reason"] = format!("unsafe or oversized {name}").into();
                return Ok(false);
            }
        }
        let source = ordinary(&project.join("source.rs"), 64 * 1024)?;
        let readme = ordinary(&project.join("README.md"), 4096)?;
        result["artifact_sha256"] = digest(&source).into();
        if readme != corpus::README.as_bytes() {
            result["reason"] = "protected README changed".into();
            return Ok(false);
        }
        fs::write(input.join("source.rs"), &source)?;
        fs::write(input.join("README.md"), &readme)?;
        fs::write(
            input.join("main.rs"),
            if kind == "a" {
                corpus::A_ORACLE
            } else {
                corpus::B_ORACLE
            },
        )?;
        let probe = capture(
            sandbox(&input, &build, tools, vec!["/usr/bin/true".into()]),
            out,
        )
        .await?;
        require(probe.code == 0, "Oracle sandbox unavailable.")?;
        let compile = receipt(
            capture(
                sandbox(
                    &input,
                    &build,
                    tools,
                    vec![
                        "/helper".into(),
                        "compile".into(),
                        tools["compiler"].as_str().unwrap().into(),
                        tools["linker"].as_str().unwrap().into(),
                    ],
                ),
                out,
            )
            .await?,
        )?;
        fs::write(out.join("compile.log"), &compile.1)?;
        if compile.0 != 0 {
            result["reason"] = "compilation failed".into();
            return Ok(false);
        }
        // Runtime sees only its compiled binary and private scratch, plus the
        // read-only toolchain/system runtime. Host expected outputs stay private.
        let executable = out.join("executable");
        fs::create_dir(&executable)?;
        fs::copy(build.join("candidate"), executable.join("candidate"))?;
        let executed = receipt(
            capture(
                sandbox(
                    &executable,
                    &runtime,
                    tools,
                    vec!["/helper".into(), "execute".into()],
                ),
                out,
            )
            .await?,
        )?;
        fs::write(out.join("stdout"), &executed.1)?;
        result["reason"] = if executed.0 == 0 && executed.1 == corpus::expected(kind) {
            "behavior matched"
        } else {
            "behavior mismatch"
        }
        .into();
        Ok(executed.0 == 0 && executed.1 == corpus::expected(kind))
    }
    .await;
    match evaluated {
        Ok(correct) => {
            result["correct"] = correct.into();
            result["status"] = if correct { "passed" } else { "failed" }.into();
        }
        Err(error) => {
            if error.code == "oracle-candidate-limit" {
                result["correct"] = false.into();
                result["status"] = "failed".into();
            }
            result["reason"] = error.message.into();
        }
    }
    result["seconds"] = start.elapsed().as_secs_f64().into();
    if out.is_dir() {
        let _ = write_json(&out.join("receipt.json"), &result);
    }
    result
}
pub async fn controls(root: &Path, tools: &V) -> Result<V> {
    fs::create_dir(root)?;
    // Neither a plausible partial patch nor an early successful process exit
    // may pass. These are controller-owned controls, not model observations.
    let cases = [
        ("a-baseline", "a", corpus::A_BASE.to_owned(), false),
        ("b-baseline", "b", corpus::B_BASE.to_owned(), false),
        ("a-gold", "a", corpus::A_GOLD.to_owned(), true),
        ("b-gold", "b", corpus::B_GOLD.to_owned(), true),
        ("a-partial", "a", corpus::partial_a(), false),
        (
            "b-exit-zero",
            "b",
            "pub fn parse_duration(_: &str) -> Result<u64,String> { std::process::exit(0) }"
                .to_owned(),
            false,
        ),
        (
            "b-binary-output", "b",
            "pub fn parse_duration(_: &str) -> Result<u64,String> { use std::io::Write; let mut out=std::io::stdout(); out.write_all(&vec![0;64*1024]).unwrap(); out.flush().unwrap(); std::process::exit(0) }".to_owned(),
            false,
        ),
    ];
    let mut rows = vec![];
    for (name, kind, source, correct) in cases {
        let project = root.join(name);
        fs::create_dir(&project)?;
        fs::write(project.join("source.rs"), source)?;
        fs::write(project.join("README.md"), corpus::README)?;
        let observed = verify(&project, &root.join(format!("{name}-oracle")), kind, tools).await;
        rows.push(json!({"name":name,"expected_correct":correct,"matched":observed["correct"]==correct,"observed":observed}));
    }
    for name in [
        "missing-source",
        "missing-readme",
        "protected-readme",
        "extra-file",
        "oversized-source",
        "linked-source",
        "hardlinked-source",
        "missing-compiler",
        "missing-linker",
    ] {
        let project = root.join(name);
        fs::create_dir(&project)?;
        fs::write(project.join("source.rs"), corpus::B_GOLD)?;
        fs::write(project.join("README.md"), corpus::README)?;
        let mut selected_tools = tools.clone();
        match name {
            "missing-source" => fs::remove_file(project.join("source.rs"))?,
            "missing-readme" => fs::remove_file(project.join("README.md"))?,
            "protected-readme" => fs::write(project.join("README.md"), b"changed")?,
            "extra-file" => fs::write(project.join("unexpected"), b"changed")?,
            "oversized-source" => fs::write(project.join("source.rs"), vec![b' '; 64 * 1024 + 1])?,
            "linked-source" => {
                fs::remove_file(project.join("source.rs"))?;
                #[cfg(unix)]
                std::os::unix::fs::symlink("README.md", project.join("source.rs"))?;
            }
            "hardlinked-source" => {
                fs::remove_file(project.join("source.rs"))?;
                fs::hard_link(project.join("README.md"), project.join("source.rs"))?;
            }
            "missing-compiler" => selected_tools["compiler"] = "/unavailable/rustc".into(),
            // An unusable toolchain is an unavailable oracle, never a failed candidate.
            "missing-linker" => selected_tools["linker"] = "/unavailable/cc".into(),
            _ => unreachable!(),
        }
        let observed = verify(
            &project,
            &root.join(format!("{name}-oracle")),
            "b",
            &selected_tools,
        )
        .await;
        let expected = if ["missing-compiler", "missing-linker"].contains(&name) {
            V::Null
        } else {
            false.into()
        };
        rows.push(json!({"name":name,"expected_correct":expected,"matched":observed["correct"]==expected,"observed":observed}));
    }
    Ok(json!({"passed":rows.iter().all(|r|r["matched"]==true),"cases":rows}))
}
