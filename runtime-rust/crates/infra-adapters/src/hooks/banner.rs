use super::{Context, project_root};
use crate::TailTrim;
use crate::{counters, output::Output, process};
use infra_core::{Result, RuntimeConfig, RuntimeContext, RuntimeError, fs, git};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
fn git_output(scope: &Context, args: &[&str]) -> Option<Vec<u8>> {
    git::run(
        &scope.roots.project,
        &git::strings(args),
        None,
        Duration::from_secs(2),
        8 * 1024 * 1024,
    )
    .ok()
    .filter(|result| result.code == 0)
    .map(|result| result.stdout)
}
fn cache_key(scope: &Context) -> Option<String> {
    let head = git_output(scope, &["rev-parse", "HEAD"])?;
    let status = git_output(
        scope,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )?;
    let diff = git_output(scope, &["diff", "HEAD"])?;
    let mut all = head;
    all.extend(status);
    all.extend(diff);
    Some(counters::checksum(&all).to_string())
}
fn cache_dir(scope: &Context) -> PathBuf {
    std::env::temp_dir().join(format!(
        "ai-accelerator-session-cache-{}",
        counters::repo_key(&scope.roots.state)
    ))
}
fn cached(scope: &Context, name: &str, key: Option<&str>) -> Option<String> {
    let key = key?;
    let directory = cache_dir(scope);
    if !counters::owned(&directory) {
        return None;
    }
    let path = directory.join(format!("{name}-{key}"));
    if !counters::owned(&path) {
        return None;
    }
    fs::read_text(&path).ok().map(|value| value.tail().into())
}
fn cache(scope: &Context, name: &str, key: Option<&str>, value: &str) {
    let Some(key) = key else {
        return;
    };
    let operation = (|| -> Result<()> {
        let directory = cache_dir(scope);
        fs::ensure_directory(&directory, true)?;
        if !counters::owned(&directory) {
            return Err(RuntimeError::policy("Foreign session cache"));
        }
        for entry in std::fs::read_dir(&directory)?.filter_map(std::result::Result::ok) {
            if counters::owned(&entry.path())
                && entry
                    .metadata()
                    .and_then(|meta| meta.modified())
                    .ok()
                    .and_then(|at| SystemTime::now().duration_since(at).ok())
                    .is_some_and(|age| age > Duration::from_secs(7 * 86400))
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        fs::atomic_write(
            &directory.join(format!("{name}-{key}")),
            format!("{value}\n").as_bytes(),
        )
    })();
    let _ = operation;
}
fn stale(root: &Path, at: SystemTime) -> bool {
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if fs::linked(&meta) {
            continue;
        }
        if meta.is_file() {
            if path.extension().is_some_and(|extension| extension == "md")
                && meta.modified().is_ok_and(|modified| modified > at)
            {
                return true;
            }
        } else if meta.is_dir()
            && let Ok(entries) = std::fs::read_dir(path)
        {
            stack.extend(
                entries
                    .filter_map(std::result::Result::ok)
                    .map(|entry| entry.path()),
            );
        }
    }
    false
}
fn php_version(scope: &Context) -> Option<String> {
    process::run(
        Path::new("php"),
        &["-r".into(), "echo PHP_VERSION;".into()],
        &scope.roots.project,
        &[],
        Duration::from_secs(2),
        &scope.environment(),
    )
    .ok()
    .filter(|output| output.code == 0)
    .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
}
pub fn render(scope: &Context) -> Result<Output> {
    let _ = counters::reset(&project_root(&scope.roots.project), &scope.payload);
    // The PHP probe, the cache key, the status and the branch are separate
    // processes with no effect on each other: they run together, most of all
    // to hide PHP's start, and their lines keep their places.
    std::thread::scope(|threads| {
        let php = threads.spawn(|| php_version(scope));
        let key = threads.spawn(|| cache_key(scope));
        let status = threads.spawn(|| scope.json("status", &[], &[]).ok());
        let branch = threads.spawn(|| git_output(scope, &["branch", "--show-current"]));
        render_lines(
            scope,
            || php.join().ok().flatten(),
            || key.join().ok().flatten(),
            || status.join().ok().flatten(),
            || branch.join().ok().flatten(),
        )
    })
}
/// The policy and skills name the memory CLI by its path from the root, which
/// a worktree this installation serves has none of, and which a session
/// started below the root does not reach: the session hears the engine that
/// works from where it is. Run there, it finds the memory and the task itself.
fn memory_cli(scope: &Context) -> Option<String> {
    if scope.roots.attached() && !scope.roots.shared {
        return None;
    }
    let place = if scope.roots.shared {
        (scope.roots.state == scope.roots.tooling
            && !super::installed_engine(&scope.roots.project).is_file())
        .then_some("this worktree has none")
    } else {
        let root = infra_core::fs::canonicalize(&scope.roots.tooling).ok()?;
        super::valid_path(scope.payload["cwd"].as_str(), 4096)
            .and_then(|cwd| infra_core::fs::canonicalize(cwd).ok())
            .filter(|cwd| cwd != &root && cwd.starts_with(&root))
            .map(|_| "this session started below the project root")
    }?;
    let engine = super::installed_engine(&scope.roots.tooling);
    if !engine.is_file() {
        return None;
    }
    // Bare, or in single quotes: the dispatcher's allow for the memory CLI
    // takes no double-quoted path, which the shell expands.
    let name = engine.to_string_lossy();
    let spelled = if name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_./-".contains(c))
    {
        name.into_owned()
    } else {
        infra_core::native::quote(&name)
    };
    Some(format!(
        "Memory CLI: {place} - run the commands the policy and skills name with `{spelled} memory` in place of `memory-bank/bin/infra-engine memory`."
    ))
}
fn render_lines(
    scope: &Context,
    php: impl FnOnce() -> Option<String>,
    key: impl FnOnce() -> Option<String>,
    status: impl FnOnce() -> Option<Value>,
    branch: impl FnOnce() -> Option<Vec<u8>>,
) -> Result<Output> {
    let mut lines = vec!["Project Context".into(), "===============".into()];
    if let Ok(version) = std::fs::read_to_string(scope.roots.tooling.join("VERSION")) {
        let version = version.split_whitespace().collect::<String>();
        let policy = fs::read_text(&scope.roots.tooling.join(".accelerator-policy-lock.json"))
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|value| {
                value["policy_digest"]
                    .as_str()
                    .map(|value| value.chars().take(8).collect::<String>())
            });
        lines.push(format!(
            "Accelerator version: {version}{}",
            policy
                .map(|digest| format!(" (policy {digest})"))
                .unwrap_or_default()
        ));
    }
    // A worktree the installation serves shares its state, so it is no
    // attached project: the retired banner told the two apart by whether the
    // state lies outside the installation.
    if scope.roots.shared {
        lines.push(format!("Accelerator: the installation in {}, whose memory this worktree shares (Git gives the worktree none of it): the memory-bank/ and project-brain/ paths the policy and skills name are there.",scope.roots.tooling.display()));
    } else if scope.roots.attached() {
        lines.push(format!(
            "Accelerator: attached from {} (nothing is installed in this project).",
            scope.roots.tooling.display()
        ));
        lines.push(format!(
            "Accelerator state (Project Brain, Memory Bank, index): {}",
            scope.roots.state.display()
        ));
    }
    if let Some(line) = memory_cli(scope) {
        lines.push(line);
    }
    let managers = [
        ("composer.json", "composer"),
        ("composer.lock", "composer.lock"),
        ("package-lock.json", "npm"),
        ("yarn.lock", "yarn"),
        ("pnpm-lock.yaml", "pnpm"),
    ]
    .into_iter()
    .filter_map(|(file, name)| scope.roots.project.join(file).is_file().then_some(name))
    .collect::<Vec<_>>();
    if !managers.is_empty() {
        lines.push(format!("Dependencies: {}", managers.join(" ")));
    }
    if let Some(version) = php() {
        lines.push(format!("PHP: {version}"));
    }
    for (paths, label) in [
        (
            &["phpunit.xml", "phpunit.xml.dist"][..],
            "Tests: PHPUnit config present",
        ),
        (&["tests/Pest.php"][..], "Tests: Pest present"),
        (
            &[".php-cs-fixer.php", ".php-cs-fixer.dist.php"][..],
            "Formatter: PHP-CS-Fixer config present",
        ),
        (
            &["phpcs.xml", "phpcs.xml.dist"][..],
            "Formatter: PHP_CodeSniffer config present",
        ),
        (
            &["phpstan.neon", "phpstan.neon.dist"][..],
            "Static analysis: PHPStan config present",
        ),
        (
            &["psalm.xml", "psalm.xml.dist"][..],
            "Static analysis: Psalm config present",
        ),
        (&["rector.php"][..], "Refactoring: Rector config present"),
        (
            &["phpbench.json"][..],
            "Benchmarks: PHPBench config present",
        ),
        (
            &["public/index.php"][..],
            "Entry point: public/index.php (front controller)",
        ),
        (&["Makefile"][..], "Build system: Make"),
        (
            &["Dockerfile", "docker-compose.yml", "compose.yaml"][..],
            "Containers: Docker present",
        ),
    ] {
        if paths
            .iter()
            .any(|path| scope.roots.project.join(path).is_file())
        {
            lines.push(label.into());
        }
    }
    let configured = RuntimeConfig::load(&scope.roots)?;
    let framework = configured.value["framework"]
        .as_str()
        .unwrap_or("generic")
        .to_ascii_lowercase();
    if !framework.contains("laravel") && !framework.contains("symfony") {
        let composer = std::fs::read_to_string(scope.roots.project.join("composer.json"))
            .unwrap_or_default()
            .to_ascii_lowercase();
        let detected = if composer.contains("yiisoft/yii2") {
            Some("Yii")
        } else if composer.contains("cakephp/cakephp") {
            Some("CakePHP")
        } else if scope.roots.project.join("bin/console").is_file() {
            Some("Symfony")
        } else if scope.roots.project.join("artisan").is_file() {
            Some("Laravel")
        } else {
            None
        };
        if let Some(framework) = detected {
            lines.extend(["".into(),format!("NOTE: {framework} detected. This is the native-PHP base folder (PHP Core/)."),"      For framework-specific conventions, switch to the matching sibling folder (Laravel/, Symfony/).".into()]);
        }
    }
    let key = key();
    let status = status();
    let branch = branch();
    if let Some(status) = status {
        if let Some(branch) = branch.map(|bytes| String::from_utf8_lossy(&bytes).tail().to_string())
        {
            let uncommitted = status["uncommitted"]
                .as_u64()
                .or_else(|| {
                    git_output(scope, &["status", "--short"]).map(|bytes| {
                        bytes
                            .split(|b| *b == b'\n')
                            .filter(|line| !line.is_empty())
                            .count() as u64
                    })
                })
                .unwrap_or(0);
            let more = status["accelerator_uncommitted"].as_u64().unwrap_or(0);
            lines.push(format!(
                "Branch: {branch} ({uncommitted} uncommitted changes{})",
                if more > 0 {
                    format!("; {more} more are the accelerator's own files")
                } else {
                    String::new()
                }
            ));
        }
        let database = status["database"].as_str().map(PathBuf::from);
        let index = if database.as_ref().is_none_or(|path| !path.is_file()) {
            "missing"
        } else if status["documents"].as_u64() == Some(0) {
            "empty"
        } else {
            "healthy"
        };
        // A missing or empty index has no freshness to report.
        let staleness = database
            .as_ref()
            .filter(|_| index == "healthy")
            .and_then(|path| std::fs::metadata(path).ok())
            .and_then(|meta| meta.modified().ok())
            .map(|at| {
                let mut paths = [
                    "AGENTS.md",
                    "README.md",
                    "CHANGELOG.md",
                    "specs",
                    "docs",
                    "tasks",
                ]
                .into_iter()
                .map(|name| scope.roots.project.join(name))
                .collect::<Vec<_>>();
                paths.extend(
                    [
                        "memory-bank/chunks",
                        "project-brain/dynamic",
                        "project-brain/control",
                    ]
                    .into_iter()
                    .map(|name| scope.roots.state.join(name)),
                );
                paths.extend(
                    [
                        "AGENTS.md",
                        ".agents/skills",
                        ".claude/skills",
                        ".cursor/skills",
                    ]
                    .into_iter()
                    .map(|name| scope.roots.tooling.join(name)),
                );
                if paths.iter().any(|path| stale(path, at)) {
                    "stale"
                } else {
                    "current"
                }
            })
            .unwrap_or("unknown");
        let validation = cached(scope, "brain-validation", key.as_deref()).unwrap_or_else(|| {
            let state = scope
                .json("validate", &[], &[])
                .ok()
                .map(|value| {
                    if value["valid"] == true {
                        "valid"
                    } else {
                        "invalid"
                    }
                })
                .unwrap_or("unavailable")
                .to_string();
            if state != "unavailable" {
                cache(scope, "brain-validation", key.as_deref(), &state);
            }
            state
        });
        let mode = status["mode"].as_str().unwrap_or("unknown");
        let bindings = status["working"]
            .as_u64()
            .map(|count| count.to_string())
            .unwrap_or_else(|| "unknown".into());
        lines.push(format!("Context governance: mode={mode}, index={index}/{staleness}, active-bindings={bindings}, brain-validation={validation}."));
    } else {
        lines.push("Context governance: mode/index/bindings/validation unavailable.".into());
    }
    if scope.roots.state.join("memory-bank/INDEX.md").is_file() {
        let summary = cached(scope, "memory-summary", key.as_deref()).or_else(|| {
            let mode = configured.mode(None).ok()?;
            let actor = std::env::var("PROJECT_BRAIN_OWNER").unwrap_or_else(|_| "local".into());
            let context = RuntimeContext::new(scope.roots.clone(), actor, mode).ok()?;
            let summary = crate::bank::summary(&context).ok()?;
            cache(scope, "memory-summary", key.as_deref(), &summary);
            Some(summary)
        });
        if let Some(summary) = summary {
            if scope.roots.project == scope.roots.tooling {
                lines.push(format!("{summary} Read memory-bank/README.md and INDEX.md before relevant durable-memory work."));
            } else {
                lines.push(format!(
                    "{summary} Read {} and {} before relevant durable-memory work.",
                    scope.roots.tooling.join("memory-bank/README.md").display(),
                    scope.roots.state.join("memory-bank/INDEX.md").display()
                ));
            }
        } else {
            lines.push("Memory bank: available (validation unavailable).".into());
        }
    }
    lines.extend(["".into(), "Structure:".into()]);
    // The session's own host tree closes the list, as each host's hook did.
    let host_trees: &[&str] = match scope.host.as_str() {
        "codex" => &[".agents", ".codex"],
        "cursor" => &[".cursor"],
        _ => &[".claude"],
    };
    for name in [
        "src",
        "app",
        "public",
        "config",
        "bin",
        "templates",
        "views",
        "tests",
        "specs",
        "tasks",
        "memory-bank",
        "project-brain",
        "examples",
    ]
    .iter()
    .chain(host_trees)
    {
        let path = scope.roots.project.join(name);
        if path.is_dir() {
            let count = std::fs::read_dir(path)?
                .filter_map(std::result::Result::ok)
                .filter(|entry| entry.path().is_file())
                .count();
            lines.push(format!("  {name}/ ({count} files)"));
        }
    }
    for (name, label) in [("tasks", "Tasks"), ("specs", "Specs")] {
        let path = scope.roots.project.join(name);
        if path.is_dir() {
            let count = std::fs::read_dir(path)?
                .filter_map(std::result::Result::ok)
                .filter(|entry| {
                    if name == "tasks" {
                        entry.path().is_dir()
                            && entry.file_name().to_string_lossy().starts_with("TASK-")
                    } else {
                        entry.path().is_file()
                            && entry.path().extension().is_some_and(|e| e == "md")
                            && entry.file_name() != "MANIFEST.md"
                    }
                })
                .count();
            lines.push(format!("  {label}: {count}"));
        }
    }
    if scope.roots.state == scope.roots.tooling {
        let _ =
            crate::dispatch::install_for_session(&scope.roots.tooling, &scope.host, &scope.payload);
    }
    let stderr = super::memory::session_cursor(scope)?;
    Ok(Output {
        code: 0,
        stdout: format!("{}\n", lines.join("\n")).into_bytes(),
        stderr,
        ..Default::default()
    })
}
