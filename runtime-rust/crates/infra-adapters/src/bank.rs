use crate::output::Output;
use infra_core::{
    BankService, Result, RuntimeContext,
    bank::{codec, publication::PublicationFiles},
    domain, git,
};
use serde_json::json;
pub fn summary(context: &RuntimeContext) -> Result<String> {
    let service = BankService::new(context.clone());
    let paths = codec::paths(&service)?;
    let (mut total, mut active, mut needs, mut expired, mut gone) = (0, 0, 0, 0, 0);
    let today = domain::clock::local_today();
    let mut files = PublicationFiles::new(&context.roots.state)?;
    for path in paths {
        total += 1;
        let chunk = match codec::read(&mut files, &path) {
            Ok(chunk) => chunk,
            Err(_) => continue,
        };
        let metadata = chunk.metadata;
        if metadata["status"] == "active" {
            active += 1;
            let missing = metadata["sources"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .filter(|source| {
                    !source.starts_with("http://")
                        && !source.starts_with("https://")
                        && domain::source_path_problem(source).is_none()
                })
                .any(|source| {
                    std::fs::metadata(context.roots.resolve(source))
                        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
                });
            if missing {
                gone += 1;
            }
        }
        if metadata["status"] == "needs-review" {
            needs += 1;
        }
        if metadata["valid_to"]
            .as_str()
            .and_then(domain::python_date)
            .is_some_and(|date| date < today)
        {
            expired += 1;
        }
    }
    let mut summary = format!("Memory bank: {total} chunks ({active} active, {needs} needs review");
    if expired > 0 {
        summary.push_str(&format!(", {expired} past valid_to)."));
    } else {
        summary.push_str(").");
    }
    if gone > 0 {
        summary.push_str(&format!(" {gone} active chunk(s) cite a file that is gone and are not retrieved; memory-bank/bin/infra-engine bank validate names them."));
    }
    Ok(summary)
}
/// Where Git moved a cited file that is gone - the end of its rename chain,
/// when that file exists. For the person repointing a chunk; the per-prompt
/// checks never start Git for this.
fn moved_to(project: &std::path::Path, source: &str) -> Option<String> {
    let mut current = source.to_string();
    for _ in 0..5 {
        let result = git::run(
            project,
            &git::strings(&[
                "log",
                "-M",
                "--diff-filter=R",
                "--name-status",
                "--format=",
                "-n",
                "1",
                "--full-diff",
                "--",
                &current,
            ]),
            None,
            std::time::Duration::from_secs(5),
            1024 * 1024,
        )
        .ok()?;
        let stdout = String::from_utf8_lossy(&result.stdout);
        let target = stdout
            .lines()
            .filter(|line| line.starts_with('R'))
            .map(|line| line.split('\t').collect::<Vec<_>>())
            .find(|parts| parts.len() == 3 && parts[1] == current)
            .map(|parts| parts[2].to_string());
        let (0, Some(target)) = (result.code, target) else {
            break;
        };
        current = target;
        if project.join(&current).is_file() {
            return Some(current);
        }
    }
    None
}
pub fn execute(
    context: RuntimeContext,
    operation: &str,
    request: &serde_json::Value,
) -> Result<Output> {
    let service = BankService::new(context.clone());
    match operation {
        "validate" if request["summary"] == true => {
            Ok(Output::text(format!("{}\n", summary(&context)?)))
        }
        "validate" if request["json"] == true => {
            let errors = service.validate()?;
            let mut output = Output::json(&json!({"valid":errors.is_empty(),"errors":errors}))?;
            output.code = i32::from(!errors.is_empty());
            Ok(output)
        }
        "validate" => {
            let (errors, warnings) = service.report()?;
            // A chunk citing a file renamed with `git mv` is left out of every
            // capsule; say where the file went, so the chunk can be repointed.
            let located = |line: String| match line
                .rsplit_once("source path does not exist: ")
                .map(|(_, source)| source.trim())
                .filter(|source| !source.is_empty())
                .and_then(|source| moved_to(&context.roots.project, source))
            {
                Some(target) => format!("{line} - Git moved it to {target}"),
                None => line,
            };
            // Warnings print on both paths and never fail: they describe a
            // chunk doing what it promised.
            let mut stderr = String::new();
            if !warnings.is_empty() {
                stderr.push_str(&format!(
                    "Memory bank validation warnings ({}):\n",
                    warnings.len()
                ));
                for warning in warnings {
                    stderr.push_str(&format!("- {}\n", located(warning)));
                }
            }
            let mut output = if errors.is_empty() {
                Output::text("Memory bank validation passed.\n")
            } else {
                stderr.push_str(&format!(
                    "Memory bank validation failed ({} error(s)):\n",
                    errors.len()
                ));
                for error in errors {
                    stderr.push_str(&format!("- {}\n", located(error)));
                }
                Output {
                    code: 1,
                    ..Default::default()
                }
            };
            output.stderr = stderr.into_bytes();
            Ok(output)
        }
        "summary" => Ok(Output::text(format!("{}\n", summary(&context)?))),
        "index" => Output::json(&service.index()?),
        "audit" => Output::json(&service.audit(request["provenance"] == true)?),
        _ => Err(infra_core::RuntimeError::invalid("Unknown bank command")),
    }
}
