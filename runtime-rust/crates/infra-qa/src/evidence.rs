use crate::{common::*, manifest};
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
pub fn catalog(v: &Value) -> Result<Vec<Value>> {
    schema_version(v)?;
    let cases = v["cases"].as_array().context("case catalog cases")?;
    if cases.len() != 18 {
        bail!("catalog requires exactly eighteen cases");
    }
    let runs = cases
        .iter()
        .map(|c| text(&c["run_id"]))
        .collect::<BTreeSet<_>>();
    let ids = cases
        .iter()
        .map(|c| text(&c["test_case_id"]))
        .collect::<BTreeSet<_>>();
    if runs != codex().into_iter().collect()
        || ids != (1..=18).map(|n| format!("TC-AI-{n:03}")).collect()
    {
        bail!("catalog identities differ");
    }
    for c in cases {
        if !["deterministic", "hybrid", "native"]
            .contains(&c["classification"].as_str().unwrap_or(""))
        {
            bail!("invalid case classification");
        }
        let commands = c["commands"].as_array().context("commands must be array")?;
        if c["classification"] == "deterministic" && commands.is_empty() && c["blocker"].is_null() {
            bail!("deterministic case lacks commands or blocker");
        }
        let mut seen = BTreeSet::new();
        for cmd in commands {
            if !seen.insert(text(&cmd["id"])) {
                bail!("duplicate command ID");
            }
            let argv = cmd["argv"].as_array().context("argv must be array")?;
            if argv.is_empty() || argv.iter().any(|v| !v.is_string()) {
                bail!("argv must be nonempty string array");
            }
        }
    }
    Ok(cases.clone())
}
pub fn summary(items: &[Value]) -> Value {
    json!({"executed":items.len(),"passed":items.iter().filter(|v|v["status"]=="passed").count(),"failed":items.iter().filter(|v|v["status"]=="failed").count(),"skipped":items.iter().filter(|v|v["status"]=="skipped").count(),"items":items})
}
pub fn status(s: &Value) -> &'static str {
    if s["failed"].as_u64().unwrap_or(0) > 0 {
        "failed"
    } else if s["skipped"].as_u64().unwrap_or(0) > 0 {
        "skipped"
    } else {
        "passed"
    }
}
fn assertion(id: &str, status: &str, message: String) -> Value {
    json!({"id":id,"status":status,"message":message})
}
fn python_command(argv: &[String]) -> bool {
    argv.iter().any(|s| {
        s == "{python}"
            || s.contains("{python_shell}")
            || Regex::new(r"(?i)(^|[^\w/.-])python(?:3(?:\.\d+)?)?(?:\s|$)")
                .unwrap()
                .is_match(s)
    })
}
pub fn migrate_catalog(catalogue: &Value, mappings: &Value) -> Result<Value> {
    catalog(catalogue)?;
    if schema_version(catalogue)? != LEGACY_VERSION {
        bail!("only a legacy catalog can be migrated");
    }
    let map = mappings
        .as_object()
        .context("migration mappings must be an object keyed run_id/command_id")?;
    let mut out = catalogue.clone();
    let mut used = BTreeSet::new();
    for case in out["cases"].as_array_mut().unwrap() {
        let run = text(&case["run_id"]);
        for cmd in case["commands"].as_array_mut().unwrap() {
            let key = format!("{run}/{}", text(&cmd["id"]));
            let old = cmd["argv"]
                .as_array()
                .unwrap()
                .iter()
                .map(text)
                .collect::<Vec<_>>();
            if let Some(v) = map.get(&key) {
                let replacement = v.as_array().context("mapped argv must be string array")?;
                if replacement.is_empty() || replacement.iter().any(|v| !v.is_string()) {
                    bail!("invalid mapped argv");
                }
                let replacement = replacement.iter().map(text).collect::<Vec<_>>();
                if python_command(&replacement) {
                    bail!("native catalog mapping still invokes Python");
                }
                cmd["argv"] = json!(replacement);
                used.insert(key);
            } else if python_command(&old) {
                bail!("explicit native argv mapping required for {key}");
            }
        }
    }
    if map.keys().any(|k| !used.contains(k)) {
        bail!("unused native command mapping");
    }
    out["schema_version"] = NATIVE_VERSION.into();
    catalog(&out)?;
    Ok(out)
}
fn executable(name: &str) -> bool {
    if name.contains(std::path::MAIN_SEPARATOR) {
        return regular(Path::new(name));
    }
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).any(|p| {
        let q = p.join(name);
        if regular(&q) {
            return true;
        }
        #[cfg(windows)]
        {
            for ext in [".exe", ".cmd", ".bat"] {
                if regular(&p.join(format!("{name}{ext}"))) {
                    return true;
                }
            }
        }
        false
    })
}
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
fn expanded(argv: &Value, root: &Path) -> Result<Vec<String>> {
    let exe = std::env::current_exe()?.to_string_lossy().into_owned();
    let root = root.to_string_lossy();
    let mut out = vec![];
    for a in argv.as_array().context("argv array")? {
        let a = a.as_str().context("argv string")?;
        out.push(if a == "{infra}" {
            exe.clone()
        } else if a == "{root}" {
            root.to_string()
        } else {
            a.replace("{infra_shell}", &shell_quote(&exe))
                .replace("{root_shell}", &shell_quote(&root))
        });
    }
    if python_command(&out) {
        bail!("native QA runner refuses Python commands; migrate the catalog explicitly");
    }
    Ok(out)
}
fn sanitize(argv: &[String], root: &Path) -> Result<Vec<String>> {
    let mut repl = vec![
        (root.to_string_lossy().into_owned(), "{root}".to_string()),
        (
            std::env::current_exe()?.to_string_lossy().into_owned(),
            "{infra}".into(),
        ),
    ];
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        repl.push((home.to_string_lossy().into_owned(), "~".into()));
    }
    repl.sort_by_key(|p| std::cmp::Reverse(p.0.len()));
    Ok(argv
        .iter()
        .map(|s| {
            let mut s = s.clone();
            for (from, to) in &repl {
                s = s.replace(from, to);
            }
            s
        })
        .collect())
}
pub fn run_case(
    root: &Path,
    output: &Path,
    case: &Value,
    hosts: &BTreeSet<String>,
) -> Result<PathBuf> {
    let selected = crate::catalog::for_version(root, NATIVE_VERSION)?;
    run_case_with_catalog(root, output, case, hosts, &selected)
}
pub fn run_case_with_catalog(
    root: &Path,
    output: &Path,
    case: &Value,
    hosts: &BTreeSet<String>,
    selected: &crate::catalog::Snapshot,
) -> Result<PathBuf> {
    if selected.version != NATIVE_VERSION {
        bail!("native evidence requires a native catalog");
    }
    let run = case["run_id"].as_str().context("run ID")?;
    if selected.case(run)? != case {
        bail!("case differs from selected catalog snapshot");
    }
    let destination = output.join(run);
    if fs::symlink_metadata(&destination).is_ok() {
        bail!("evidence directory exists; refusing overwrite");
    }
    fs::create_dir_all(output)?;
    let tmp = tempfile::Builder::new()
        .prefix(".qa-run-")
        .tempdir_in(output)?;
    let dir = tmp.path();
    fs::create_dir(dir.join("stdout"))?;
    fs::create_dir(dir.join("stderr"))?;
    let before = git(root, &["status", "--porcelain=v1"])?;
    let started = now();
    let mut commands = vec![];
    let mut assertions = vec![];
    let mut missing = vec![];
    for p in case["required_test_patterns"]
        .as_array()
        .context("required patterns")?
    {
        let glob = p["glob"].as_str().context("glob string")?;
        let re = Regex::new(p["regex"].as_str().context("regex string")?)?;
        let mut found = false;
        for p in glob::glob(&root.join(glob).to_string_lossy())? {
            let p = p?;
            if regular(&p) && re.is_match(&String::from_utf8_lossy(&fs::read(&p)?)) {
                found = true;
                break;
            }
        }
        if !found {
            missing.push(format!("{glob} / {}", text(&p["regex"])));
        }
    }
    let defs = case["commands"].as_array().context("commands array")?;
    if !missing.is_empty() {
        assertions.push(assertion(
            "required-regression-discovery",
            "skipped",
            format!(
                "Required focused regression tests are not discoverable: {}",
                missing.join("; ")
            ),
        ));
    } else if defs.is_empty() && !case["blocker"].is_null() {
        assertions.push(assertion(
            "declared-blocker",
            "skipped",
            text(&case["blocker"]),
        ));
    } else {
        for (index, c) in defs.iter().enumerate() {
            let id = c["id"].as_str().context("command id")?;
            let cwd = root.join(c["cwd"].as_str().context("command cwd")?);
            if !cwd.is_dir() {
                assertions.push(assertion(
                    id,
                    "skipped",
                    format!("Command cwd does not exist: {}", text(&c["cwd"])),
                ));
                continue;
            }
            let cwd = crate::spelling::canonicalize(cwd)?;
            // The recorded cwd is relative to this same spelling of the root:
            // a short Windows name or a linked temporary root reads otherwise.
            let base = crate::spelling::canonicalize(root)?;
            if !cwd.starts_with(&base) {
                bail!("command cwd escapes repository");
            }
            if let Some(required) = c["requires_executable"].as_str()
                && !executable(required)
            {
                assertions.push(assertion(
                    id,
                    "skipped",
                    format!("Required executable is not configured: {required}"),
                ));
                continue;
            }
            let argv = expanded(&c["argv"], root)?;
            let began = now();
            let result = Command::new(&argv[0])
                .args(&argv[1..])
                .current_dir(&cwd)
                .output()?;
            let ended = now();
            let code = exit_code(&result.status);
            let stdout = format!("stdout/{:02}-{id}.txt", index + 1);
            let stderr = format!("stderr/{:02}-{id}.txt", index + 1);
            atomic(&dir.join(&stdout), &result.stdout)?;
            atomic(&dir.join(&stderr), &result.stderr)?;
            let relative_cwd = cwd
                .strip_prefix(&base)?
                .to_string_lossy()
                .replace('\\', "/");
            let recorded_cwd = if relative_cwd.is_empty() {
                "."
            } else {
                relative_cwd.as_str()
            };
            commands.push(json!({"id":id,"argv":sanitize(&argv,root)?,"cwd":recorded_cwd,"started_at":began,"ended_at":ended,"exit_code":code,"stdout_path":stdout,"stderr_path":stderr}));
            assertions.push(assertion(
                id,
                if result.status.success() {
                    "passed"
                } else {
                    "failed"
                },
                format!("exit code {code}"),
            ));
        }
    }
    let required = case["native_requirement"]
        .as_str()
        .filter(|s| !s.is_empty());
    let native = if let Some(required) = required {
        let required = required
            .split(',')
            .map(str::trim)
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let missing = required.difference(hosts).cloned().collect::<Vec<_>>();
        json!({"required":true,"status":"skipped","reason":if missing.is_empty(){"Native host availability was declared, but this deterministic runner cannot manufacture native-client observations; attach reviewed native evidence separately.".into()}else{format!("Native host unavailable: {}",missing.join(", "))}})
    } else {
        json!({"required":false,"status":"not-applicable","reason":null})
    };
    if native["required"] == true {
        assertions.push(assertion(
            "native-client-observation",
            "skipped",
            text(&native["reason"]),
        ));
    }
    if assertions.is_empty() {
        assertions.push(assertion(
            "empty-case-guard",
            "failed",
            "Case executed no commands and declared no blocker".into(),
        ));
    }
    let after = git(root, &["status", "--porcelain=v1"])?;
    let sum = summary(&assertions);
    let evidence = json!({"schema_version":NATIVE_VERSION,"run_id":case["run_id"],"test_case_id":case["test_case_id"],"classification":case["classification"],"started_at":started,"ended_at":now(),"status":status(&sum),"commands":commands,"assertions":sum,"native":native});
    let branch = git(root, &["branch", "--show-current"])?;
    let branch = if branch.trim().is_empty() {
        format!(
            "detached at {}",
            git(root, &["rev-parse", "--short", "HEAD"])?.trim()
        )
    } else {
        branch.trim().to_owned()
    };
    let commit = git(root, &["rev-parse", "HEAD"])?.trim().to_owned();
    write_json(&dir.join("case.json"), case)?;
    write_json(
        &dir.join("environment.json"),
        &json!({"captured_at":now(),"platform":format!("{}-{}",std::env::consts::OS,std::env::consts::ARCH),"runtime":"infra-qa-rust","runtime_version":env!("CARGO_PKG_VERSION"),"artifact_contract":NATIVE_VERSION,"executable":"{infra}","repository_root":"{root}","repository_commit":commit,"branch":branch}),
    )?;
    write_json(&dir.join("commands.json"), &json!(commands))?;
    let exits = commands
        .iter()
        .map(|c| (text(&c["id"]), c["exit_code"].clone()))
        .collect::<serde_json::Map<_, _>>();
    write_json(&dir.join("exit-codes.json"), &Value::Object(exits))?;
    write_json(&dir.join("assertions.json"), &sum)?;
    write_json(&dir.join("run.json"), &evidence)?;
    atomic(&dir.join("git-before.txt"), before.as_bytes())?;
    atomic(&dir.join("git-after.txt"), after.as_bytes())?;
    let files = manifest::artifacts(dir, &dir.join("SHA256SUMS.txt"))?
        .iter()
        .map(|p| {
            p.strip_prefix(dir)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<Vec<_>>();
    write_json(
        &dir.join("inventory.json"),
        &json!({"repository_status_changed":before!=after,"files":files}),
    )?;
    write_json(
        &dir.join("provenance.json"),
        &json!({"catalog":selected.source,"catalog_sha256":selected.sha256,"runner":"infra qa run","artifact_contract":NATIVE_VERSION,"repository_commit":commit,"generated_at":now(),"historical_claim_used_as_pass_evidence":false}),
    )?;
    atomic(
        &dir.join("SHA256SUMS.txt"),
        manifest::render(dir, &dir.join("SHA256SUMS.txt"))?.as_bytes(),
    )?;
    if fs::symlink_metadata(&destination).is_ok() {
        bail!("evidence directory appeared during run");
    }
    let kept = tmp.keep();
    fs::rename(&kept, &destination).inspect_err(|_| {
        let _ = fs::remove_dir_all(&kept);
    })?;
    Ok(destination)
}
fn exit_code(s: &std::process::ExitStatus) -> i32 {
    if let Some(c) = s.code() {
        return c;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        -s.signal().unwrap_or(1)
    }
    #[cfg(not(unix))]
    {
        -1
    }
}
pub fn run(root: &Path, args: &[String]) -> Result<i32> {
    let (o, p) = parse_args(
        args,
        &["--cases", "--output-root", "--run-id", "--native-host"],
        &["--list"],
    )?;
    if !p.is_empty() {
        bail!("unexpected positional argument");
    }
    let cases_path = PathBuf::from(option(
        &o,
        "--cases",
        &crate::catalog::native_path(root).to_string_lossy(),
    ));
    let selected_catalog = crate::catalog::Snapshot::read(root, &cases_path)?;
    let cases = selected_catalog.cases.clone();
    let selected = o
        .get("--run-id")
        .cloned()
        .unwrap_or_else(codex)
        .into_iter()
        .collect::<BTreeSet<_>>();
    if !selected.is_subset(&codex().into_iter().collect()) {
        bail!("unknown Run ID");
    }
    let cases = cases
        .into_iter()
        .filter(|c| selected.contains(&text(&c["run_id"])))
        .collect::<Vec<_>>();
    if o.contains_key("--list") {
        for c in cases {
            println!(
                "{} {} {}",
                text(&c["run_id"]),
                text(&c["classification"]),
                text(&c["title"])
            );
        }
        return Ok(0);
    }
    if selected_catalog.version != NATIVE_VERSION {
        bail!(
            "executing legacy Python catalogs is disabled; migrate-catalog with reviewed command mappings"
        );
    }
    let hosts = o
        .get("--native-host")
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect::<BTreeSet<_>>();
    if hosts
        .iter()
        .any(|s| !["claude-code", "cursor", "codex"].contains(&s.as_str()))
    {
        bail!("unsupported native host");
    }
    let output = PathBuf::from(option(
        &o,
        "--output-root",
        &root.join(crate::catalog::NATIVE_EVIDENCE).to_string_lossy(),
    ));
    crate::catalog::preflight(&output, &cases)?;
    for c in &cases {
        run_case_with_catalog(root, &output, c, &hosts, &selected_catalog)?;
    }
    println!("Wrote evidence for {} case(s)", cases.len());
    Ok(0)
}
