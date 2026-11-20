//! `skillcache-tool`: one program of a capsule, run under the program contract.
//!
//! A program gets a filtered environment with its inputs in
//! `SKILLCACHE_INPUT`, runs in the task workspace within its time limit, and
//! answers with exactly one JSON object on standard output:
//! `{"status": "complete", "result": {...}}`, where `result` holds every
//! declared output. Anything else is a failure with a bounded, redacted
//! diagnostic; the package it ran from is verified unchanged afterwards.
use crate::{
    capsule::{BINDING_BYTES, Program},
    env_filter,
    package::{self, Package},
};
use infra_core::{Result, RuntimeError};
use infra_host::capture::{self, Stream};
use serde_json::{Map, Value};
use std::{
    path::Path,
    time::{Duration, Instant},
};

/// Standard output is one JSON object; this bounds it.
pub const STDOUT_BYTES: usize = 256 * 1024;
/// Both streams together, before the run is stopped.
pub const OUTPUT_BYTES: usize = 8 * 1024 * 1024;
pub const DIAGNOSTIC_BYTES: usize = 4 * 1024;

pub enum Mode<'a> {
    /// Run `entry` with these inputs.
    Run(&'a Map<String, Value>),
    /// Run `check`: the program's own test, without task files.
    Check,
}

#[derive(Clone, Debug)]
pub struct Completed {
    pub result: Map<String, Value>,
    pub seconds: f64,
}

#[derive(Clone, Debug)]
pub struct Failed {
    /// `nonzero_exit`, `timeout`, `missing_output`, `invalid_stdout`,
    /// `output_limit`, `runtime_missing`, `package_modified` or `unavailable`.
    pub reason: &'static str,
    pub exit_code: Option<i32>,
    /// The end of standard error, with secrets masked.
    pub diagnostics: String,
    /// Declared outputs the result did not carry.
    pub missing: Vec<String>,
    pub seconds: f64,
}
impl Failed {
    fn new(reason: &'static str, diagnostics: impl Into<String>) -> Self {
        Self {
            reason,
            exit_code: None,
            diagnostics: diagnostics.into(),
            missing: Vec::new(),
            seconds: 0.0,
        }
    }
    /// A second attempt can only help an ordinary non-zero exit.
    pub fn retryable(&self) -> bool {
        self.reason == "nonzero_exit"
    }
}

pub enum Outcome {
    Completed(Completed),
    Failed(Failed),
}

fn redacted(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim();
    match infra_core::domain::mask_secrets(text) {
        Ok((masked, _)) => masked,
        Err(_) => "[diagnostics withheld: a possible secret could not be masked]".into(),
    }
}

fn package_path(package: &Package, text: &str) -> bool {
    (text.starts_with("programs/") || text.starts_with("assets/")) && package.files.contains(text)
}

/// The command to start: the runtime found on `PATH`, package files named by
/// their absolute path.
fn command(package: &Package, arguments: &[String]) -> std::result::Result<Vec<String>, Failed> {
    let Some(runtime) = arguments.first() else {
        return Err(Failed::new("unavailable", "The program has no command."));
    };
    if !crate::capsule::is_runtime(runtime) {
        return Err(Failed::new(
            "unavailable",
            "The program's runtime is not a bare interpreter name.",
        ));
    }
    let Some(executable) = infra_host::provider::find_executable(runtime) else {
        return Err(Failed::new(
            "runtime_missing",
            format!("Runtime `{runtime}` is not installed on this machine."),
        ));
    };
    if executable
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        })
    {
        return Err(Failed::new(
            "runtime_missing",
            format!("Runtime `{runtime}` resolves to a batch launcher, which is never started."),
        ));
    }
    let Some(executable) = executable.to_str() else {
        return Err(Failed::new(
            "runtime_missing",
            "The runtime's path is not UTF-8.",
        ));
    };
    let absolute = |relative: &str| package.root.join(relative).display().to_string();
    let mut argv = vec![executable.to_string()];
    for argument in &arguments[1..] {
        argv.push(match argument.split_once('=') {
            _ if package_path(package, argument) => absolute(argument),
            Some((option, value)) if package_path(package, value) => {
                format!("{option}={}", absolute(value))
            }
            _ => argument.clone(),
        });
    }
    Ok(argv)
}

/// Reads a program's standard output: one JSON object and nothing else.
fn answer(stdout: &[u8], declared: &[String]) -> std::result::Result<Map<String, Value>, Failed> {
    let refused = |message: &str| {
        Failed::new(
            "invalid_stdout",
            format!(
                "{message} Expected exactly `{{\"status\": \"complete\", \"result\": {{...}}}}` on standard output."
            ),
        )
    };
    if stdout.len() > STDOUT_BYTES {
        return Err(refused("Standard output exceeds its limit."));
    }
    let text = std::str::from_utf8(stdout)
        .map_err(|_| refused("Standard output is not UTF-8."))?
        .trim();
    let value = infra_core::strict_json::parse(text.as_bytes(), STDOUT_BYTES, 16, 20_000)
        .map_err(|_| refused("Standard output is not one JSON object."))?;
    let object = value
        .as_object()
        .ok_or_else(|| refused("Standard output is not a JSON object."))?;
    if object.len() != 2 || object.get("status").and_then(Value::as_str) != Some("complete") {
        return Err(refused("The answer has other members or another status."));
    }
    let result = object
        .get("result")
        .and_then(Value::as_object)
        .ok_or_else(|| refused("`result` is not a JSON object."))?;
    let mut missing = Vec::new();
    for name in declared {
        match result.get(name) {
            None | Some(Value::Null) => missing.push(name.clone()),
            Some(value)
                if serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
                    > BINDING_BYTES =>
            {
                missing.push(name.clone());
            }
            Some(_) => {}
        }
    }
    if !missing.is_empty() {
        let mut failed = Failed::new(
            "missing_output",
            format!(
                "The result lacks declared output(s) {} (each a value of at most {BINDING_BYTES} bytes).",
                missing.join(", ")
            ),
        );
        failed.missing = missing;
        return Err(failed);
    }
    Ok(result.clone())
}

/// Runs one program and reads its answer. An `Err` is a failure of this
/// engine's own bookkeeping; whatever the program did wrong is an
/// [`Outcome::Failed`].
pub fn run(
    package: &Package,
    program: &Program,
    mode: Mode<'_>,
    workspace: &Path,
) -> Result<Outcome> {
    let (arguments, input, declared): (&[String], String, &[String]) = match mode {
        Mode::Run(inputs) => {
            let mut selected = Map::new();
            for name in &program.inputs {
                let value = inputs.get(name).ok_or_else(|| {
                    RuntimeError::invalid(format!(
                        "Input `{name}` of `{}` is not bound.",
                        program.id
                    ))
                })?;
                selected.insert(name.clone(), value.clone());
            }
            (
                &program.entry,
                serde_json::to_string(&Value::Object(selected))?,
                &program.outputs,
            )
        }
        Mode::Check => (&program.check, "{}".into(), &[]),
    };
    let argv = match command(package, arguments) {
        Ok(argv) => argv,
        Err(failed) => return Ok(Outcome::Failed(failed)),
    };
    let before = package::digest(&package.root)?;
    let mut environment =
        env_filter::program_environment(std::env::vars_os(), &input, workspace, &package.root);
    // An interpreter must not write its cache into the read-only package.
    environment.insert("PYTHONDONTWRITEBYTECODE".into(), "1".into());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let started = Instant::now();
    let ran = capture::run(
        capture::Request {
            argv: &argv,
            cwd: workspace,
            environment: &environment,
            input: &[],
            output_limit: OUTPUT_BYTES,
            deadline: started + Duration::from_secs(program.timeout_seconds),
        },
        || false,
        |_| {},
        |stream, bytes| {
            match stream {
                Stream::Stdout => {
                    let room = (STDOUT_BYTES + 1).saturating_sub(stdout.len());
                    stdout.extend_from_slice(&bytes[..bytes.len().min(room)]);
                }
                Stream::Stderr => {
                    stderr.extend_from_slice(bytes);
                    if stderr.len() > 2 * DIAGNOSTIC_BYTES {
                        let cut = stderr.len() - DIAGNOSTIC_BYTES;
                        stderr.drain(..cut);
                    }
                }
            }
            Ok(())
        },
    );
    let seconds = started.elapsed().as_secs_f64();
    if stderr.len() > DIAGNOSTIC_BYTES {
        let cut = stderr.len() - DIAGNOSTIC_BYTES;
        stderr.drain(..cut);
    }
    let diagnostics = redacted(&stderr);
    let failed = |reason: &'static str, message: String, exit_code: Option<i32>| {
        Ok(Outcome::Failed(Failed {
            reason,
            exit_code,
            diagnostics: if diagnostics.is_empty() {
                message
            } else {
                format!("{message}\n{diagnostics}")
            },
            missing: Vec::new(),
            seconds,
        }))
    };
    let exit = match ran {
        Ok(outcome) => outcome,
        Err(error) if error.code == "native-timeout" => {
            return failed(
                "timeout",
                format!(
                    "The program did not finish within {} s and was stopped.",
                    program.timeout_seconds
                ),
                None,
            );
        }
        Err(error) if error.code == "output-limit" => {
            return failed(
                "output_limit",
                format!("The program printed more than {OUTPUT_BYTES} bytes and was stopped."),
                None,
            );
        }
        Err(error) => {
            return failed(
                "unavailable",
                format!("The program could not be run: {error}"),
                None,
            );
        }
    };
    if package::digest(&package.root)? != before {
        return failed(
            "package_modified",
            "The capsule's package changed while the program ran; it is read-only.".into(),
            exit.exit,
        );
    }
    if exit.exit != Some(0) {
        return failed(
            "nonzero_exit",
            match exit.signal {
                Some(signal) => format!("The program was stopped by signal {signal}."),
                None => format!("The program exited with code {}.", exit.shell_code),
            },
            Some(exit.shell_code),
        );
    }
    match answer(&stdout, declared) {
        Ok(result) => Ok(Outcome::Completed(Completed { result, seconds })),
        Err(mut refused) => {
            refused.exit_code = Some(0);
            refused.seconds = seconds;
            Ok(Outcome::Failed(refused))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn tool_rejects_multiple_json_or_text() {
        let ok = br#" {"status":"complete","result":{"rows_file":"out/rows.json","invoices":12}} "#;
        let result = answer(ok, &declared(&["rows_file"])).unwrap();
        assert_eq!(result["invoices"], 12);
        for stdout in [
            &br#"{"status":"complete","result":{}}{"status":"complete","result":{}}"#[..],
            b"working...\n{\"status\":\"complete\",\"result\":{}}",
            b"{\"status\":\"complete\",\"result\":{}}\ndone",
            b"",
            b"[]",
            br#"{"status":"complete"}"#,
            br#"{"status":"failed","result":{}}"#,
            br#"{"status":"complete","result":{},"extra":1}"#,
            br#"{"status":"complete","result":[]}"#,
            br#"{"status":"complete","status":"complete","result":{}}"#,
        ] {
            let refused = answer(stdout, &[]).unwrap_err();
            assert_eq!(
                refused.reason,
                "invalid_stdout",
                "{}",
                String::from_utf8_lossy(stdout)
            );
        }
        let huge = format!(
            "{{\"status\":\"complete\",\"result\":{{\"pad\":\"{}\"}}}}",
            "x".repeat(STDOUT_BYTES)
        );
        assert_eq!(
            answer(huge.as_bytes(), &[]).unwrap_err().reason,
            "invalid_stdout"
        );
    }

    #[test]
    fn a_result_without_a_declared_output_is_refused() {
        let refused = answer(
            br#"{"status":"complete","result":{"rows_file":null,"total":"1"}}"#,
            &declared(&["rows_file", "summary_file"]),
        )
        .unwrap_err();
        assert_eq!(refused.reason, "missing_output");
        assert_eq!(refused.missing, ["rows_file", "summary_file"]);
        // An output is a value for later programs, not a document.
        let large = format!(
            "{{\"status\":\"complete\",\"result\":{{\"rows_file\":\"{}\"}}}}",
            "x".repeat(BINDING_BYTES + 1)
        );
        let refused = answer(large.as_bytes(), &declared(&["rows_file"])).unwrap_err();
        assert_eq!(refused.missing, ["rows_file"]);
    }
}
