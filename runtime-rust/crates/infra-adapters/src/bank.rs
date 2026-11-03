use crate::output::Output;
use infra_core::{
    BankService, Result, RuntimeContext,
    bank::{codec, publication::PublicationFiles},
    domain,
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
        "validate" => {
            let errors = service.validate()?;
            let code = i32::from(!errors.is_empty());
            let mut output = if request["json"] == true {
                Output::json(&json!({"valid":errors.is_empty(),"errors":errors}))?
            } else if errors.is_empty() {
                Output::text("Memory bank validation passed.\n")
            } else {
                Output::text(format!("{}\n", errors.join("\n")))
            };
            output.code = code;
            Ok(output)
        }
        "summary" => Ok(Output::text(format!("{}\n", summary(&context)?))),
        "index" => Output::json(&service.index()?),
        "audit" => Output::json(&service.audit(request["provenance"] == true)?),
        _ => Err(infra_core::RuntimeError::invalid("Unknown bank command")),
    }
}
