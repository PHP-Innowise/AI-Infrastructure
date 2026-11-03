//! Real graph-baseline processes run in an explicit guard-capable fixture.
//! The temporary Cargo package is std-only and implements capability/argv receipts.
use infra_core::bank::publication::PublicationFiles;
use infra_eval::artifact;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const BASELINE_ENGINE_LIMIT: usize = 512 * 1024 * 1024;
const BASELINE_ENGINE_NAME: &str = if cfg!(windows) {
    "infra-engine.exe"
} else {
    "infra-engine"
};
const RECEIPT_LIMIT: usize = 256 * 1024;
const DRIVER: &str = env!("CARGO_BIN_EXE_infra-eval-process-fixture");

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "infra-graph-baseline-{}",
            uuid::Uuid::new_v4().simple()
        ));
        artifact::private_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn expected_paths(base: &Path) -> Vec<PathBuf> {
    vec![
        base.join("runtime-rust/target")
            .join(infra_tooling::artifacts::native_target().unwrap())
            .join("release")
            .join(BASELINE_ENGINE_NAME),
        base.join("runtime-rust/target/release")
            .join(BASELINE_ENGINE_NAME),
        base.join("runtime-rust/bin").join(BASELINE_ENGINE_NAME),
        base.join("bin").join(BASELINE_ENGINE_NAME),
    ]
}
#[derive(Debug)]
struct Failure {
    message: String,
    kind: String,
}
fn invoke(scenario: &str, args: &[String], environment: &BTreeMap<String, String>) -> (i32, Value) {
    let mut child = Command::new(DRIVER)
        .arg(scenario)
        .args(args)
        .envs(environment)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the explicitly compiled guard-capable fixture");
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let output = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(RECEIPT_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .unwrap();
        bytes
    });
    let errors = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr
            .take(RECEIPT_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .unwrap();
        bytes
    });
    let deadline = Instant::now() + Duration::from_secs(120);
    let (status, timed_out) = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break (status, false);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            break (child.wait().unwrap(), true);
        }
        thread::sleep(Duration::from_millis(5));
    };
    let stdout = output.join().unwrap();
    let stderr = errors.join().unwrap();
    assert!(
        !timed_out,
        "guard-capable graph fixture exceeded its deadline"
    );
    assert!(stdout.len() <= RECEIPT_LIMIT && stderr.len() <= RECEIPT_LIMIT);
    assert!(stderr.is_empty(), "{}", String::from_utf8_lossy(&stderr));
    let code = status.code().unwrap_or(-1);
    assert!(
        [0, 2].contains(&code),
        "fixture code={code}, output={}",
        String::from_utf8_lossy(&stdout)
    );
    let reply: Value = serde_json::from_slice(&stdout).expect("one fixture JSON receipt");
    assert_eq!(reply["ok"], code == 0, "{reply}");
    (code, reply)
}
fn native_baseline_environment(
    base: &Path,
    runtime: Option<&str>,
    build: bool,
    environment: &BTreeMap<String, String>,
) -> Result<PathBuf, Failure> {
    let mut args = vec!["--base".into(), base.to_string_lossy().into_owned()];
    if let Some(runtime) = runtime {
        args.extend(["--runtime".into(), runtime.into()]);
    }
    if build {
        args.push("--build".into());
    }
    let (code, reply) = invoke("graph-baseline", &args, environment);
    if code == 0 {
        Ok(PathBuf::from(
            reply["path"].as_str().expect("admitted artifact path"),
        ))
    } else {
        Err(Failure {
            message: reply["error"]["message"]
                .as_str()
                .expect("fixed failure message")
                .into(),
            kind: reply["error"]["kind"]
                .as_str()
                .expect("typed failure kind")
                .into(),
        })
    }
}
fn native_baseline(base: &Path, runtime: Option<&str>, build: bool) -> Result<PathBuf, Failure> {
    native_baseline_environment(base, runtime, build, &BTreeMap::new())
}

fn fixture(clock: bool, rewrite_source: bool) -> Root {
    let root = Root::new();
    artifact::private_dir(&root.0.join("install/inventories")).unwrap();
    artifact::fresh_file(
        &root.0.join("runtime-rust/Cargo.toml"),
        br#"[package]
name = "graph-baseline-fixture"
version = "0.1.0"
edition = "2024"

[workspace]

[[bin]]
name = "infra-engine"
path = "src/main.rs"
"#,
        0o600,
    )
    .unwrap();
    artifact::fresh_file(
        &root.0.join("runtime-rust/Cargo.lock"),
        b"version = 4\n\n[[package]]\nname = \"graph-baseline-fixture\"\nversion = \"0.1.0\"\n",
        0o600,
    )
    .unwrap();
    let descriptor = json!({
        "engine":"infra-engine",
        "version":"0.1.0",
        "capability_profile":"infra-engine-v1",
        "capabilities":if clock {
            vec!["memory","brain","bank","hooks","mcp","install","memory-eval-clock-v1"]
        } else {
            vec!["memory","brain","bank","hooks","mcp","install"]
        }
    })
    .to_string();
    let mutation = if rewrite_source {
        format!(
            "std::fs::write({:?}, b\"changed compiler source\").unwrap();",
            expected_paths(&root.0)[0].to_string_lossy()
        )
    } else {
        String::new()
    };
    let main = format!(
        r#"fn main() {{
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() == 2 && args[0] == "capabilities" && args[1] == "--json" {{
        {mutation}
        println!("{{}}", {descriptor:?});
    }} else if args.first().map(String::as_str) == Some("__memory-eval-runtime")
        && args.get(1).map(String::as_str) == Some("--clock")
        && args.get(3).map(String::as_str) == Some("--")
        && args.get(4).map(String::as_str) == Some("memory")
    {{
        println!("{{}}", args.join("|"));
    }} else {{
        std::process::exit(64);
    }}
}}
"#
    );
    artifact::fresh_file(
        &root.0.join("runtime-rust/src/main.rs"),
        main.as_bytes(),
        0o600,
    )
    .unwrap();
    // The build must override both Cargo configuration selectors. A compiler
    // for this unrelated target is neither installed nor needed by the fixture.
    artifact::fresh_file(
        &root.0.join(".cargo/config.toml"),
        b"[build]\ntarget = \"wasm32-unknown-unknown\"\ntarget-dir = \"config-output\"\n",
        0o600,
    )
    .unwrap();
    root
}
#[test]
fn graph_baseline_build_ignores_cargo_output_selectors_and_preserves_dispatch() {
    let root = fixture(true, false);
    let poison = Root::new();
    let environment = BTreeMap::from([
        (
            "CARGO_TARGET_DIR".into(),
            poison
                .0
                .join("environment-output")
                .to_string_lossy()
                .into_owned(),
        ),
        ("CARGO_BUILD_TARGET".into(), "wasm32-unknown-unknown".into()),
    ]);
    let runtime = native_baseline_environment(&root.0, None, true, &environment).unwrap();
    assert!(!poison.0.join("environment-output").exists());
    let compiler_output = expected_paths(&root.0)[0].clone();
    assert_ne!(runtime, compiler_output);
    assert!(runtime.starts_with(root.0.join("runtime-rust")));
    assert_eq!(runtime.file_name().unwrap(), BASELINE_ENGINE_NAME);
    let (bytes, mode) = artifact::read_file(&runtime, BASELINE_ENGINE_LIMIT).unwrap();
    let parent = compiler_output.parent().unwrap();
    let mut source = PublicationFiles::new(parent).unwrap();
    let (source_bytes, _) = source
        .capture_source(BASELINE_ENGINE_NAME, BASELINE_ENGINE_LIMIT)
        .unwrap()
        .unwrap();
    assert_eq!(bytes, source_bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(fs::metadata(&runtime).unwrap().nlink(), 1);
        let alias = root.0.join("retained-cargo-hardlink");
        fs::hard_link(&compiler_output, &alias).unwrap();
        assert!(fs::metadata(&compiler_output).unwrap().nlink() >= 2);
    }
    assert!(!root.0.join("config-output").exists());
    assert!(!root.0.join("runtime-rust/config-output").exists());

    let selected = native_baseline(&root.0, None, false).unwrap();
    assert_ne!(selected, compiler_output);
    assert_eq!(
        artifact::read_file(&selected, BASELINE_ENGINE_LIMIT)
            .unwrap()
            .0,
        bytes
    );
    let selected = native_baseline(&root.0, compiler_output.to_str(), false).unwrap();
    assert_ne!(selected, compiler_output);
    assert_eq!(
        artifact::read_file(&selected, BASELINE_ENGINE_LIMIT)
            .unwrap()
            .0,
        bytes
    );

    let cache = Root::new();
    let (driver_code, reply) = invoke(
        "graph-runtime-at",
        &[
            "--base".into(),
            root.0.to_string_lossy().into_owned(),
            "--cache".into(),
            cache.0.to_string_lossy().into_owned(),
            "--runtime".into(),
            runtime.to_string_lossy().into_owned(),
        ],
        &BTreeMap::new(),
    );
    assert_eq!(driver_code, 0);
    assert_eq!(reply["code"], 0);
    assert!(
        reply["seconds"]
            .as_f64()
            .is_some_and(|value| value.is_finite() && value >= 0.0)
    );
    assert_eq!(
        reply["stdout"].as_str().unwrap().trim(),
        format!(
            "__memory-eval-runtime|--clock|2026-01-02T03:04:05+00:00|--|memory|--root|{}|index|--json",
            root.0.display()
        )
    );

    for index in 0..4 {
        let candidate_root = fixture(true, false);
        let candidate = expected_paths(&candidate_root.0)[index].clone();
        artifact::fresh_file(&candidate, &bytes, mode).unwrap();
        #[cfg(unix)]
        fs::hard_link(&candidate, candidate_root.0.join("source-hardlink")).unwrap();
        let selected = native_baseline(&candidate_root.0, None, false).unwrap();
        assert_ne!(selected, candidate);
        assert_eq!(
            artifact::read_file(&selected, BASELINE_ENGINE_LIMIT)
                .unwrap()
                .0,
            bytes
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(fs::metadata(selected).unwrap().nlink(), 1);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let linked = root.0.join("linked-engine");
        symlink(&runtime, &linked).unwrap();
        assert!(native_baseline(&root.0, linked.to_str(), false).is_err());
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o600)).unwrap();
        let error = native_baseline(&root.0, runtime.to_str(), false).unwrap_err();
        assert_eq!(error.message, "native baseline is not executable");
    }
}

#[test]
fn graph_baseline_rejects_successful_build_without_required_clock_protocol() {
    let root = fixture(false, false);
    let error = native_baseline(&root.0, None, true).unwrap_err();
    assert!(expected_paths(&root.0)[0].is_file());
    assert_eq!(
        error.message,
        "native baseline lacks the supported memory evaluation clock protocol"
    );
}

#[test]
fn graph_baseline_rejects_source_rewrite_during_private_capability_probe() {
    let root = fixture(true, true);
    let error = native_baseline(&root.0, None, true).unwrap_err();
    assert_eq!(
        fs::read(expected_paths(&root.0)[0].clone()).unwrap(),
        b"changed compiler source"
    );
    assert_eq!(error.kind, "Conflict");
    assert_eq!(
        error.message,
        "native baseline source changed during admission"
    );
}
