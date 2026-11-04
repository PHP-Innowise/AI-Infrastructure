//! Native ports of the attributed skill-creator utilities and model routing evaluator.
//! Provider commands execute only when the corresponding public operation is requested.
pub mod benchmark;
pub mod common;
pub mod evaluation;
pub mod optimization;
pub mod platform_cli;
pub mod reports;
pub mod review;
pub mod routing;
pub mod scaffold;
pub mod utils;
pub mod validation;
pub mod workbook_preview;
pub use infra_core::{Result, RuntimeError};
use serde_json::{Value, json};
fn dispatch(operation: &str, request: &Value) -> Result<Value> {
    let operation = operation
        .strip_prefix("skill-tools/")
        .unwrap_or(operation)
        .replace('_', "-");
    match operation.as_str() {
        "quick-validate" => {
            let skill = common::positional(request, "skill_path", 0)?;
            let (valid, message) = validation::validate_skill(std::path::Path::new(&skill))?;
            Ok(common::cli(
                if valid { 0 } else { 1 },
                format!("{message}\n"),
                "",
                json!({"valid":valid,"message":message}),
            ))
        }
        "utils" => {
            let skill = utils::parse_skill_md(std::path::Path::new(common::required(
                request,
                "skill_path",
            )?))?;
            common::json_cli(
                json!({"name":skill.name,"description":skill.description,"content":skill.content}),
                String::new(),
            )
        }
        "init-skill" | "package-skill" => scaffold::execute(&operation, request),
        "aggregate-benchmark" => benchmark::execute(request),
        "generate-report" => reports::execute(request),
        "run-eval" => evaluation::execute(request),
        "improve-description" => optimization::improve_execute(request),
        "run-loop" => optimization::loop_execute(request),
        "generate-review" => review::execute(request),
        "eval-set-review" => review::eval_set_execute(request),
        "platform-cli" => platform_cli::execute(request),
        "routing-eval" => routing::execute(request),
        _ => Err(RuntimeError::invalid(format!(
            "Unknown skill helper operation: {operation}"
        ))),
    }
}
pub fn execute(operation: &str, request: &Value) -> Result<Value> {
    let _signal = infra_eval::cancel::Scope::install()?;
    match dispatch(operation, request) {
        Ok(value) => Ok(value),
        Err(error) => {
            let code = if infra_eval::cancel::interrupted() {
                130
            } else if error.kind == infra_core::ErrorKind::InvalidInput {
                2
            } else {
                1
            };
            Ok(common::cli(
                code,
                "",
                format!("{error}\n"),
                json!({"error":error.message,"kind":format!("{:?}",error.kind)}),
            ))
        }
    }
}
