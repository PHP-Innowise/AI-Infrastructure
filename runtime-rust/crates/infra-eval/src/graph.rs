//! Synthetic source-link pilot: exact committed native runtime versus working native runtime.
use crate::executable;
use crate::{
    Result, artifact, control,
    reconstruction::{self, Reconstructor},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Duration,
};
const SOURCE: &str = "specs/rounding.md";
const FIRST: &str = "memory-bank/chunks/MEM-20260801-bbbbbbbb-minor-units.md";
const SECOND: &str = "memory-bank/chunks/MEM-20260801-cccccccc-refunds.md";
const FUTURE: &str = "memory-bank/chunks/MEM-20260810-dddddddd-reserve.md";
const DECOY: &str = "memory-bank/chunks/MEM-20260801-eeeeeeee-dashboard.md";
const FIRST_ANSWER: &str = "Floating point never touches a stored invoice total.";
const SECOND_ANSWER: &str = "A refund can never exceed the stored invoice total.";
const FUTURE_ANSWER: &str = "The reserve must be settled before the second transfer.";
fn write(root: &Path, name: &str, text: &str) -> Result<()> {
    artifact::fresh_file(
        &root.join(artifact::relative(name)?),
        text.as_bytes(),
        0o644,
    )
}
fn chunk(path: &str, title: &str, created: &str, source: &str, body: &str) -> Result<String> {
    let id = Path::new(path)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .split('-')
        .take(3)
        .collect::<Vec<_>>()
        .join("-");
    let meta = json!({"id":id,"title":title,"type":"convention","status":"active","scope":["application"],"tags":["billing"],"created":created,"last_verified":created,"review_after":"2027-08-01","sources":[source],"supersedes":[],"superseded_by":null});
    Ok(format!(
        "---\n{}\n---\n\n# {title}\n\n{body}\n",
        infra_core::domain::json::serialize(&meta, false, false, true, false)?
    ))
}
fn commit(root: &Path, when: &str, message: &str) -> Result<()> {
    let at = reconstruction::instant(when).ok_or_else(|| artifact::invalid("fixture timestamp"))?;
    let env = reconstruction::runtime_env(&std::env::temp_dir(), at, true)?;
    if !root.join(".git").exists() {
        reconstruction::commit_corpus(root, &env, None, None, "main")?;
    } else {
        let spec = crate::process::CommandSpec {
            argv: vec![
                "git".into(),
                "-c".into(),
                "user.name=Graph fixture".into(),
                "-c".into(),
                "user.email=fixture@example.invalid".into(),
                "add".into(),
                "specs".into(),
                "memory-bank".into(),
            ],
            cwd: root.into(),
            env: env.clone(),
            input: vec![],
        };
        if crate::process::capture(&spec, Duration::from_secs(30), 1024 * 1024)?.0 != 0 {
            return Err(artifact::invalid("fixture Git add failed"));
        }
        let mut spec = spec;
        spec.argv = vec![
            "git".into(),
            "-c".into(),
            "user.name=Graph fixture".into(),
            "-c".into(),
            "user.email=fixture@example.invalid".into(),
            "-c".into(),
            "commit.gpgsign=false".into(),
            "commit".into(),
            "-q".into(),
            "-m".into(),
            message.into(),
        ];
        if crate::process::capture(&spec, Duration::from_secs(30), 1024 * 1024)?.0 != 0 {
            return Err(artifact::invalid("fixture Git commit failed"));
        }
    }
    Ok(())
}
fn skills(root: &Path) -> Result<Vec<String>> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
        if !dir.is_dir() {
            return Ok(());
        }
        for e in std::fs::read_dir(dir)? {
            let e = e?;
            let m = e.file_type()?;
            if m.is_symlink() {
                return Err(artifact::invalid("skill symlink"));
            }
            if m.is_dir() {
                walk(base, &e.path(), out)?;
            } else if e.path().extension().is_some_and(|e| e == "md") {
                out.push(
                    e.path()
                        .strip_prefix(base)
                        .map_err(|_| artifact::invalid("skill root"))?
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
        Ok(())
    }
    let mut names = vec![];
    walk(root, &root.join(".agents/skills"), &mut names)?;
    names.sort();
    Ok(names)
}
fn materialize(source: &Path, target: &Path, commit: &str) -> Result<()> {
    let r = Reconstructor::new(
        source,
        &target.with_extension("cache"),
        &std::env::current_exe()?,
    )?;
    r.materialize(source, commit, target)
}
const BASELINE_ENGINE_LIMIT: usize = 512 * 1024 * 1024;
const BASELINE_ENGINE_NAME: &str = if cfg!(windows) {
    "infra-engine.exe"
} else {
    "infra-engine"
};
fn baseline_paths(base: &Path) -> Result<Vec<PathBuf>> {
    let engine = BASELINE_ENGINE_NAME;
    Ok(vec![
        base.join("runtime-rust/target")
            .join(infra_tooling::artifacts::native_target()?)
            .join("release")
            .join(engine),
        base.join("runtime-rust/target/release").join(engine),
        base.join("runtime-rust/bin").join(engine),
        base.join("bin").join(engine),
    ])
}
fn baseline_build_spec(base: &Path) -> Result<crate::process::CommandSpec> {
    Ok(crate::process::CommandSpec {
        argv: vec![
            "cargo".into(),
            "build".into(),
            "--locked".into(),
            "--release".into(),
            "--manifest-path".into(),
            base.join("runtime-rust/Cargo.toml")
                .to_string_lossy()
                .into(),
            "--bin".into(),
            "infra-engine".into(),
            "--target".into(),
            infra_tooling::artifacts::native_target()?,
            "--target-dir".into(),
            base.join("runtime-rust/target").to_string_lossy().into(),
        ],
        cwd: base.into(),
        // The process runner clears its environment. Retain the actual Cargo,
        // rustup and compiler configuration; the explicit output/target flags
        // bind the expected artifact independently of inherited Cargo selectors.
        env: reconstruction::clean_env(),
        input: vec![],
    })
}
fn admit_baseline(base: &Path, runtime: &Path) -> Result<PathBuf> {
    use infra_core::bank::publication::{PublicationFiles, private::RootProtection};
    let runtime = artifact::ordinary(runtime)?;
    let parent = runtime
        .parent()
        .ok_or_else(|| artifact::invalid("native baseline has no parent"))?;
    let name = runtime
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| artifact::invalid("native baseline name is not UTF-8"))?;
    let mut source = PublicationFiles::new(parent)?;
    // Cargo may hardlink its selected output to deps/. Capture is read-only
    // and pins the source image; mutation readers retain one-link admission.
    let (bytes, identity) = source
        .capture_source(name, BASELINE_ENGINE_LIMIT)?
        .ok_or_else(|| artifact::invalid("native baseline artifact is absent"))?;
    if bytes.is_empty() || cfg!(unix) && identity.2 & 0o111 == 0 {
        return Err(artifact::invalid("native baseline is not executable"));
    }
    let check_source = |source: &mut PublicationFiles| -> Result<()> {
        if source
            .capture_source(name, BASELINE_ENGINE_LIMIT)?
            .is_none_or(|(after, after_identity)| after != bytes || after_identity != identity)
        {
            return Err(crate::RuntimeError::conflict(
                "native baseline source changed during admission",
            ));
        }
        source.check()
    };
    let relative = format!(
        "runtime-rust/.graph-baseline-{}",
        uuid::Uuid::new_v4().simple()
    );
    let mut owner = PublicationFiles::new(base)?;
    owner.create_private_directory_exclusive(&relative)?;
    let directory = base.join(&relative);
    let mut held = PublicationFiles::open_directory_root(&directory, RootProtection::SelectedRoot)?;
    let mode = if cfg!(windows) { 0o600 } else { 0o500 };
    held.write_expected_mode_checked(
        BASELINE_ENGINE_NAME,
        &bytes,
        None,
        None,
        mode,
        BASELINE_ENGINE_LIMIT,
    )?;
    check_source(&mut source)?;
    // Only execute the private one-link copy of the captured compiler bytes.
    let executable = directory.join(BASELINE_ENGINE_NAME);
    let capabilities = infra_tooling::installer::Installer::new(base)?
        .with_engine(&executable)?
        .engine_capabilities()?;
    check_source(&mut source)?;
    if held
        .read(BASELINE_ENGINE_NAME, BASELINE_ENGINE_LIMIT)?
        .as_deref()
        != Some(bytes.as_slice())
        || held.read_mode(BASELINE_ENGINE_NAME)? != Some(if cfg!(windows) { 0o644 } else { mode })
    {
        return Err(crate::RuntimeError::conflict(
            "native baseline private copy changed during admission",
        ));
    }
    owner.check()?;
    if capabilities["capability_profile"] != "infra-engine-v1"
        || !capabilities["capabilities"]
            .as_array()
            .is_some_and(|values| values.iter().any(|v| v == "memory-eval-clock-v1"))
    {
        return Err(artifact::invalid(
            "native baseline lacks the supported memory evaluation clock protocol",
        ));
    }
    Ok(executable)
}
/// Select and snapshot a native graph baseline before capability admission.
pub fn native_baseline(base: &Path, override_path: Option<&str>, build: bool) -> Result<PathBuf> {
    let base = artifact::ordinary(base)?;
    let manifest = base.join("runtime-rust/Cargo.toml");
    if !manifest.is_file() {
        return Err(artifact::invalid(
            "native-baseline-unavailable: the committed reference contains no native runtime; choose a native commit",
        ));
    }
    artifact::read_file(&manifest, artifact::MAX_FILE)?;
    let candidates = baseline_paths(&base)?;
    let runtime = override_path
        .map(PathBuf::from)
        .or_else(|| candidates.iter().find(|p| p.is_file()).cloned());
    if let Some(runtime) = runtime {
        return admit_baseline(&base, &runtime);
    }
    if !build {
        return Err(artifact::invalid(
            "native-baseline-unavailable: provide --baseline-runtime or --build-baseline",
        ));
    }
    let spec = baseline_build_spec(&base)?;
    if crate::process::capture(&spec, Duration::from_secs(900), 8 * 1024 * 1024)?.0 != 0 {
        return Err(artifact::invalid("native baseline build failed"));
    }
    admit_baseline(&base, &candidates[0])
}

#[cfg(test)]
#[path = "graph_baseline_tests.rs"]
mod baseline_tests;
pub fn run(source: &Path, current: &Path, request: &Value) -> Result<Value> {
    let out = PathBuf::from(
        request["out_dir"]
            .as_str()
            .ok_or_else(|| artifact::invalid("--out-dir required"))?,
    );
    if out.exists() || artifact::overlaps(source, &out)? {
        return Err(artifact::invalid("--out-dir must be new and outside clone"));
    }
    artifact::private_dir(&out)?;
    let reference = request["baseline_ref"].as_str().unwrap_or("HEAD");
    let commit = reconstruction::git(
        source,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{reference}^{{commit}}"),
        ],
    )?;
    if commit.is_empty() {
        return Err(artifact::invalid(
            "baseline reference is not a local commit",
        ));
    }
    let baseline = out.join("baseline-runtime");
    materialize(source, &baseline, &commit)?;
    let baseline_runtime = native_baseline(
        &baseline,
        request["baseline_runtime"].as_str(),
        request["build_baseline"] == true,
    )?;
    let project = out.join("projects/billing");
    artifact::private_dir(&project)?;
    for (name, text) in [
        (
            SOURCE,
            "# Rounding\n\nAmounts use half-to-even banker tie breaking.\n",
        ),
        (
            "specs/delivery.md",
            "# Delivery\n\nQuartz shipment customs boundary: check customs before releasing the shipment.\n",
        ),
        (
            "specs/reserve.md",
            "# Reserve\n\nThe heliotrope reserve escrow has a separate settlement policy.\n",
        ),
        (
            "specs/format.md",
            "# Format\n\nAmber representation encoding serialization requires UTF-8.\n",
        ),
    ] {
        write(&project, name, text)?;
    }
    for (name, title, body, cite) in [
        (FIRST, "Minor units", FIRST_ANSWER, SOURCE),
        (SECOND, "Refund validation", SECOND_ANSWER, SOURCE),
        (
            DECOY,
            "Export dashboard",
            "Use blue headings in the export dashboard.",
            "specs/format.md",
        ),
    ] {
        write(
            &project,
            name,
            &chunk(name, title, "2026-08-01", cite, body)?,
        )?;
    }
    commit_fixture(
        &project,
        "2026-08-01T10:00:00Z",
        "Initial source-backed knowledge",
    )?;
    write(
        &project,
        FUTURE,
        &chunk(
            FUTURE,
            "Reserve consequence",
            "2026-08-10",
            "specs/reserve.md",
            FUTURE_ANSWER,
        )?,
    )?;
    commit_fixture(&project, "2026-08-10T10:00:00Z", "Future knowledge")?;
    let prompts = [
        ("source-link", "Explain half-to-even banker tie breaking."),
        (
            "direct-memory",
            "Can a refund exceed the stored invoice total?",
        ),
        ("unrelated", "Explain the quartz shipment customs boundary."),
        ("no-answer", "Explain vermilion semaphore calibration."),
        ("future", "Explain heliotrope reserve escrow."),
        (
            "source-decoy",
            "Explain amber representation encoding serialization.",
        ),
    ]
    .iter()
    .map(|(id, p)| json!({"id":id,"prompt":p,"project":"billing","ts":"2026-08-05T12:00:00Z"}))
    .collect::<Vec<_>>();
    let mut grades = json!({"source-link":{(SOURCE):1,(FIRST):2,(SECOND):2},"direct-memory":{(FIRST):1,(SECOND):2},"unrelated":{"specs/delivery.md":2},"no-answer":{},"future":{"specs/reserve.md":1,(FUTURE):2},"source-decoy":{"specs/format.md":2,(DECOY):0}});
    let names = skills(&source.join("PHP Core"))?
        .into_iter()
        .chain(skills(&baseline.join("PHP Core"))?)
        .collect::<BTreeSet<_>>();
    for g in grades.as_object_mut().unwrap().values_mut() {
        for name in &names {
            if g.get(name).is_none() {
                g[name] = 0.into();
            }
        }
    }
    let passages = json!({"source-link":{(FIRST):{"useful":true,"passages":[FIRST_ANSWER]},(SECOND):{"useful":true,"passages":[SECOND_ANSWER]}},"direct-memory":{(SECOND):{"useful":true,"passages":[SECOND_ANSWER]}},"unrelated":{"specs/delivery.md":{"useful":true,"passages":["check customs before releasing the shipment"]}},"future":{(FUTURE):{"useful":true,"passages":[FUTURE_ANSWER]}},"source-decoy":{"specs/format.md":{"useful":true,"passages":["requires UTF-8"]}}});
    for (name, value) in [
        ("set", json!(prompts)),
        ("judgments", grades),
        ("passages", passages),
    ] {
        artifact::write_json(&out.join("inputs").join(format!("{name}.json")), &value)?;
    }
    let scorer = executable::digest(current)?;
    let mut outputs = vec![];
    for (label, root, runtime) in [
        ("baseline", baseline.as_path(), baseline_runtime.as_path()),
        ("automatic", source, current),
    ] {
        if crate::cancel::interrupted() {
            break;
        }
        let output = out.join("results").join(format!("{label}.json"));
        let args = json!({"tooling_root":root,"runtime_binary":runtime,"set":out.join("inputs/set.json"),"judgments":out.join("inputs/judgments.json"),"passages":out.join("inputs/passages.json"),"projects_root":project.parent(),"edition":"PHP Core","host":"codex","cache":out.join("cache").join(label),"out":output});
        let result = control::eval_run(&args)?;
        if result["_cli"]["code"] != 0 {
            return Ok(result);
        }
        let mut measured = artifact::read_json(&output, artifact::MAX_TREE)?;
        // The evaluator already sealed the exact selected edition inventory.
        // Whole-checkout snapshots would include Cargo outputs and local state.
        let resource_identity = measured["meta"]["editions"]["PHP Core"]["resource_sha256"]
            .as_str()
            .filter(|value| artifact::hex(value, 64))
            .ok_or_else(|| artifact::invalid("graph evaluation resource identity is absent"))?
            .to_string();
        measured["meta"]["runtime_under_test"] = json!({"label":label,"commit":if label=="baseline"{commit.clone()}else{reconstruction::git(source,&["rev-parse","HEAD"])?},"materialization":if label=="baseline"{"git-blobs"}else{"working-tree"},"scorer_sha256":scorer,"runtime_sha256":executable::digest(runtime)?,"resource_identity":resource_identity});
        artifact::write_json(&output, &measured)?;
        outputs.push((output, measured));
    }
    if outputs.len() != 2 {
        return Err(artifact::invalid(
            "graph pilot interrupted; partial result files retained",
        ));
    }
    let report = crate::reports::format(
        &[
            ("A", outputs[0].0.as_path(), &outputs[0].1),
            ("B", outputs[1].0.as_path(), &outputs[1].1),
        ],
        false,
    )?;
    Ok(
        json!({"_cli":{"code":0,"stdout":format!("{report}\n"),"stderr":""},"result":{"outputs":outputs.iter().map(|r|&r.0).collect::<Vec<_>>(),"comparison":crate::reports::compatibility(&outputs[0].1,&outputs[1].1),"synthetic":true,"task_outcome_benefit":null}}),
    )
}
fn commit_fixture(root: &Path, when: &str, message: &str) -> Result<()> {
    commit(root, when, message)
}
