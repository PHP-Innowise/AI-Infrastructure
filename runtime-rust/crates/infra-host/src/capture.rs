//! Bounded split-stream capture through the shared native custody owners.
//! Complete child environments are explicit; no caller waits on pipe reader threads.
use crate::{HostError, Result};
use std::{collections::BTreeMap, path::Path, time::Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stream {
    Stdout,
    Stderr,
}
pub struct Request<'a> {
    pub argv: &'a [String],
    pub cwd: &'a Path,
    pub environment: &'a BTreeMap<String, String>,
    pub input: &'a [u8],
    pub output_limit: usize,
    pub deadline: Instant,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeExit {
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub shell_code: i32,
}
pub struct Outcome {
    /// The native exit value; a POSIX signal has no ordinary exit value.
    pub exit: Option<i32>,
    pub signal: Option<i32>,
    pub shell_code: i32,
    pub seconds: f64,
}
pub(crate) fn unconfirmed() -> HostError {
    HostError {
        status: 503,
        code: "cleanup-unconfirmed",
        message: "Native capture cleanup could not be confirmed.".into(),
    }
}
pub(crate) fn cancelled() -> HostError {
    HostError {
        status: 409,
        code: "native-cancelled",
        message: "Cancellation requested.".into(),
    }
}
pub(crate) fn timeout() -> HostError {
    HostError {
        status: 504,
        code: "native-timeout",
        message: "Native capture deadline reached.".into(),
    }
}
pub(crate) fn overflow() -> HostError {
    HostError {
        status: 503,
        code: "output-limit",
        message: "Native capture output limit reached.".into(),
    }
}
/// A scoped runtime worker permits synchronous controllers to use the same
/// cancellable native owner even when their caller already runs inside Tokio.
pub fn run(
    request: Request<'_>,
    interrupted: impl FnMut() -> bool + Send,
    observed_exit: impl FnMut(NativeExit) + Send,
    event: impl FnMut(Stream, &[u8]) -> Result<()> + Send,
) -> Result<Outcome> {
    if request.output_limit == 0 || request.output_limit > 512 * 1024 * 1024 {
        return Err(HostError::invalid("Invalid native capture byte limit."));
    }
    if Instant::now() >= request.deadline {
        return Err(timeout());
    }
    std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("infra-captured-process".into())
            .spawn_scoped(scope, move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                runtime.block_on(run_async(request, interrupted, observed_exit, event))
            })?;
        worker.join().unwrap_or_else(|_| Err(unconfirmed()))
    })
}
#[cfg(unix)]
async fn run_async(
    request: Request<'_>,
    interrupted: impl FnMut() -> bool + Send,
    observed_exit: impl FnMut(NativeExit) + Send,
    event: impl FnMut(Stream, &[u8]) -> Result<()> + Send,
) -> Result<Outcome> {
    crate::process::run_captured_until(request, interrupted, observed_exit, event).await
}
#[cfg(windows)]
#[path = "capture_windows.rs"]
mod windows;
#[cfg(windows)]
async fn run_async(
    request: Request<'_>,
    interrupted: impl FnMut() -> bool + Send,
    observed_exit: impl FnMut(NativeExit) + Send,
    event: impl FnMut(Stream, &[u8]) -> Result<()> + Send,
) -> Result<Outcome> {
    windows::run_async(request, interrupted, observed_exit, event).await
}
#[cfg(not(any(unix, windows)))]
async fn run_async(
    _: Request<'_>,
    _: impl FnMut() -> bool + Send,
    _: impl FnMut(NativeExit) + Send,
    _: impl FnMut(Stream, &[u8]) -> Result<()> + Send,
) -> Result<Outcome> {
    Err(HostError::unavailable(
        "Native capture custody is unavailable on this platform.",
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExperimentSignal {
    TerminateLeader,
    InterruptGroup,
    KillGroup,
}
#[derive(Clone, Copy, Debug)]
pub struct ExperimentStop {
    pub after: std::time::Duration,
    pub signal: ExperimentSignal,
    pub grace: std::time::Duration,
}
/// Run the provider-free lifecycle interruption experiment. Natural IPC
/// completion includes stdin closure, both output EOFs and the actual native
/// exit. An incomplete exchange receives the selected signal after `after`,
/// then gets `grace` to drain before an owned group kill.
///
/// Unix uses two isolated process groups in the caller's session: an unreaped
/// direct-child anchor reserves the native PGID, and a separate direct-child
/// observer retains the native wait status. This deliberately does not claim
/// the controlling-terminal/session semantics of `setsid`. Ordinary capture
/// and Harness execution retain their existing session layout and timing.
///
/// Non-Unix hosts refuse before creating a worker or native process. The
/// caller's absolute request deadline remains a hard upper bound for IPC.
pub fn run_experiment(
    request: Request<'_>,
    stop: ExperimentStop,
    interrupted: impl FnMut() -> bool + Send,
    observed_exit: impl FnMut(NativeExit) + Send,
    event: impl FnMut(Stream, &[u8]) -> Result<()> + Send,
) -> Result<Outcome> {
    #[cfg(not(unix))]
    {
        let _ = (request, stop, interrupted, observed_exit, event);
        Err(HostError {
            status: 503,
            code: "capture-experiment-unsupported",
            message: "Scheduled lifecycle signal experiments require Unix process groups.".into(),
        })
    }
    #[cfg(unix)]
    {
        if request.output_limit == 0
            || request.output_limit > 512 * 1024 * 1024
            || stop.grace.is_zero()
            || std::time::Instant::now()
                .checked_add(stop.after)
                .and_then(|at| at.checked_add(stop.grace))
                .is_none()
        {
            return Err(HostError::invalid("Invalid lifecycle experiment budget."));
        }
        if Instant::now() >= request.deadline {
            return Err(timeout());
        }
        std::thread::scope(|scope| {
            let worker = std::thread::Builder::new()
                .name("infra-lifecycle-experiment".into())
                .spawn_scoped(scope, move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    runtime.block_on(crate::process::run_captured_experiment(
                        request,
                        stop,
                        interrupted,
                        observed_exit,
                        event,
                    ))
                })?;
            worker.join().unwrap_or_else(|_| Err(unconfirmed()))
        })
    }
}

#[cfg(all(test, not(unix)))]
mod experiment_platform_tests {
    use super::*;
    #[test]
    fn signal_experiment_refuses_before_callbacks_or_native_creation() {
        for signal in [
            ExperimentSignal::TerminateLeader,
            ExperimentSignal::InterruptGroup,
            ExperimentSignal::KillGroup,
        ] {
            let environment = BTreeMap::new();
            let error = run_experiment(
                Request {
                    argv: &[],
                    cwd: Path::new("unavailable-experiment-fixture"),
                    environment: &environment,
                    input: &[],
                    output_limit: 1,
                    deadline: Instant::now() + std::time::Duration::from_secs(1),
                },
                ExperimentStop {
                    after: std::time::Duration::from_millis(1),
                    signal,
                    grace: std::time::Duration::from_millis(1),
                },
                || -> bool { panic!("unsupported experiment called interruption callback") },
                |_| panic!("unsupported experiment observed a native exit"),
                |_, _| -> Result<()> { panic!("unsupported experiment read native output") },
            )
            .err()
            .expect("non-Unix experiment must refuse");
            assert_eq!(error.code, "capture-experiment-unsupported");
        }
    }
}
