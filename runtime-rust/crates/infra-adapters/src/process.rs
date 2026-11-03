//! One adapter over the owned native process runner used by tooling and Harness.
//! No shell forwarding; child deadlines terminate/reap the owned process tree.
use crate::output::Output;
use infra_core::{Result, RuntimeError};
use std::{collections::BTreeMap, ffi::OsString, path::Path, time::Duration};
pub const LIMIT: usize = 4 * 1024 * 1024;
pub fn run(
    binary: &Path,
    args: &[String],
    cwd: &Path,
    input: &[u8],
    timeout: Duration,
    environment: &BTreeMap<OsString, OsString>,
) -> Result<Output> {
    let environment = environment
        .iter()
        .map(|(key, value)| (key.clone(), Some(value.clone())))
        .collect::<BTreeMap<_, _>>();
    run_with_env(binary, args, cwd, input, timeout, &environment)
}
/// A typed child environment overlay: None removes an inherited key.
pub fn run_with_env(
    binary: &Path,
    args: &[String],
    cwd: &Path,
    input: &[u8],
    timeout: Duration,
    environment: &BTreeMap<OsString, Option<OsString>>,
) -> Result<Output> {
    if timeout.is_zero() {
        return Err(RuntimeError::invalid("Native deadline must be positive"));
    }
    let arguments = args.iter().map(OsString::from).collect::<Vec<_>>();
    match infra_tooling::process::run_with_env(
        binary.as_os_str(),
        &arguments,
        cwd,
        Some(input.to_vec()),
        timeout,
        LIMIT,
        environment,
    ) {
        Ok(output) => Ok(Output {
            code: output.code,
            stdout: output.stdout,
            stderr: output.stderr,
            ..Default::default()
        }),
        Err(error) if error.code.as_deref() == Some("native-timeout") => Ok(Output {
            code: 124,
            ..Default::default()
        }),
        Err(error) => Err(error),
    }
}
pub fn engine(
    args: &[String],
    cwd: &Path,
    input: &[u8],
    timeout: Duration,
    environment: &BTreeMap<OsString, OsString>,
) -> Result<Output> {
    run(
        &std::env::current_exe()?,
        args,
        cwd,
        input,
        timeout,
        environment,
    )
}
