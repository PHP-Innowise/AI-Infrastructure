//! Owned native process groups; the parent keeps the watchdog writer.
#[cfg(unix)]
use crate::HarnessError;
use crate::Result;
use serde_json::Value;
#[cfg(unix)]
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
#[cfg(unix)]
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
#[derive(Clone)]
pub struct Limits {
    pub seconds: Option<u64>,
    pub output: usize,
    pub line: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            seconds: Some(900),
            output: 4 * 1024 * 1024,
            line: 2 * 1024 * 1024,
        }
    }
}
pub struct Output {
    pub code: i32,
    pub bytes: Vec<u8>,
    pub seconds: f64,
}
pub trait RunObserver: Send + Sync {
    fn boundary(&self, signal: &str, details: &Value) -> Result<()>;
}
#[cfg(unix)]
fn observed(observer: &Option<Arc<dyn RunObserver>>, signal: &str, details: Value) -> Result<()> {
    if let Some(observer) = observer {
        observer.boundary(signal, &details)?;
    }
    Ok(())
}
#[cfg(unix)]
#[expect(
    clippy::too_many_arguments,
    reason = "The stable native process boundary keeps runner custody and output delivery explicit."
)]
pub async fn run_with_lock(
    argv: &[String],
    cwd: &Path,
    environment: &BTreeMap<String, String>,
    input: &[u8],
    limits: Limits,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<std::fs::File>>,
    event: impl FnMut(&[u8]) -> Result<()> + Send,
) -> Result<Output> {
    run_engine(
        argv,
        cwd,
        environment,
        &[],
        input,
        limits,
        cancel,
        runner_lock,
        None,
        false,
        None,
        event,
    )
    .await
}
#[cfg(unix)]
#[expect(
    clippy::too_many_arguments,
    reason = "The stable native process boundary keeps scoped environment removal and runner custody explicit."
)]
pub async fn run_environment(
    argv: &[String],
    cwd: &Path,
    environment: &BTreeMap<String, String>,
    remove: &[String],
    input: &[u8],
    limits: Limits,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<std::fs::File>>,
    event: impl FnMut(&[u8]) -> Result<()> + Send,
) -> Result<Output> {
    run_engine(
        argv,
        cwd,
        environment,
        remove,
        input,
        limits,
        cancel,
        runner_lock,
        None,
        false,
        None,
        event,
    )
    .await
}
#[cfg(unix)]
#[expect(
    clippy::too_many_arguments,
    reason = "The stable native process boundary carries scoped environment, observer and runner custody per call."
)]
pub async fn run_monitored(
    argv: &[String],
    cwd: &Path,
    environment: &BTreeMap<String, String>,
    remove: &[String],
    input: &[u8],
    limits: Limits,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<std::fs::File>>,
    observer: Option<Arc<dyn RunObserver>>,
    event: impl FnMut(&[u8]) -> Result<()> + Send,
) -> Result<Output> {
    run_engine(
        argv,
        cwd,
        environment,
        remove,
        input,
        limits,
        cancel,
        runner_lock,
        observer,
        false,
        None,
        event,
    )
    .await
}
#[cfg(unix)]
#[expect(
    clippy::too_many_arguments,
    reason = "The stable native process boundary carries an absolute deadline and retained runner custody per call."
)]
pub async fn run_monitored_until(
    argv: &[String],
    cwd: &Path,
    environment: &BTreeMap<String, String>,
    remove: &[String],
    input: &[u8],
    limits: Limits,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<std::fs::File>>,
    observer: Option<Arc<dyn RunObserver>>,
    deadline: Instant,
    event: impl FnMut(&[u8]) -> Result<()> + Send,
) -> Result<Output> {
    run_engine(
        argv,
        cwd,
        environment,
        remove,
        input,
        limits,
        cancel,
        runner_lock,
        observer,
        false,
        Some(deadline),
        event,
    )
    .await
}
#[cfg(unix)]
#[expect(
    clippy::too_many_arguments,
    reason = "The stable raw capture boundary keeps runner custody and bounded output delivery explicit."
)]
pub async fn run_raw_with_lock(
    argv: &[String],
    cwd: &Path,
    environment: &BTreeMap<String, String>,
    input: &[u8],
    limits: Limits,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<std::fs::File>>,
    event: impl FnMut(&[u8]) -> Result<()> + Send,
) -> Result<Output> {
    run_engine(
        argv,
        cwd,
        environment,
        &[],
        input,
        limits,
        cancel,
        runner_lock,
        None,
        true,
        None,
        event,
    )
    .await
}
#[cfg(unix)]
fn emit_lines<F>(line: &mut Vec<u8>, limit: usize, event: &mut F) -> Result<()>
where
    F: FnMut(&[u8]) -> Result<()> + ?Sized,
{
    let mut consumed = 0;
    while let Some(end) = line[consumed..].iter().position(|byte| *byte == b'\n') {
        if end > limit {
            if consumed > 0 {
                drop(line.drain(..consumed));
            }
            return Err(HarnessError {
                status: 503,
                code: "output-line-limit",
                message: "Native output line limit reached.".into(),
            });
        }
        let next = consumed + end + 1;
        if let Err(error) = event(&line[consumed..next]) {
            // The old drain removed this packet before calling the observer.
            drop(line.drain(..next));
            return Err(error);
        }
        consumed = next;
    }
    if consumed > 0 {
        drop(line.drain(..consumed));
    }
    if line.len() > limit {
        return Err(HarnessError {
            status: 503,
            code: "output-line-limit",
            message: "Native output line limit reached.".into(),
        });
    }
    Ok(())
}
#[cfg(unix)]
#[expect(
    clippy::too_many_arguments,
    reason = "A single dispatch path carries immutable process options and retained custody without global state."
)]
async fn run_engine(
    argv: &[String],
    cwd: &Path,
    environment: &BTreeMap<String, String>,
    remove: &[String],
    input: &[u8],
    limits: Limits,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<std::fs::File>>,
    observer: Option<Arc<dyn RunObserver>>,
    raw: bool,
    deadline: Option<Instant>,
    mut event: impl FnMut(&[u8]) -> Result<()> + Send,
) -> Result<Output> {
    validate_command(argv, remove)?;
    if deadline.is_some_and(|until| Instant::now() >= until) {
        return Err(native_timeout());
    }
    if cancel.load(Ordering::Acquire) {
        return Err(native_cancelled());
    }
    let started = Instant::now();
    let (mut guardian, mut stdin, stdout) =
        launch_guardian(argv, cwd, environment, remove, raw, runner_lock).await?;
    let start_error = observed(
        &observer,
        "guardian_spawned",
        serde_json::json!({"guardian_pid":guardian.pid}),
    )
    .err();
    let prompt = input.to_vec();
    let watching = observer.clone();
    let admitted = start_error.is_none();
    let writer = tokio::spawn(async move {
        if admitted {
            stdin.write_all(&prompt).await?;
            observed(
                &watching,
                "input_written",
                serde_json::json!({"bytes":prompt.len()}),
            )?;
        }
        stdin.shutdown().await?;
        observed(&watching, "input_closed", Value::Null)?;
        Ok::<(), HarnessError>(())
    });
    let mut reader = BufReader::new(stdout);
    let mut output = Vec::new();
    let mut line = Vec::new();
    let execute: Result<()> = async {
        if let Some(error) = start_error {
            return Err(error);
        }
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err(native_cancelled());
            }
            if limits
                .seconds
                .is_some_and(|s| started.elapsed() > Duration::from_secs(s))
                || deadline.is_some_and(|until| Instant::now() >= until)
            {
                return Err(native_timeout());
            }
            let mut chunk = [0u8; 65536];
            let wait = deadline
                .map(|until| {
                    until
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(100))
                })
                .unwrap_or(Duration::from_millis(100));
            let read = tokio::select! {
                status = guardian.completed(), if guardian.status.is_none() => {
                    if status == GuardStatus::Unconfirmed {
                        return Err(cleanup_unconfirmed());
                    }
                    // Confirmed cleanup closes the native pipe. Drain its
                    // already-admitted bytes before reporting guardian loss.
                    continue;
                },
                read = tokio::time::timeout(wait, reader.read(&mut chunk)) => read,
            };
            let count = match read {
                Err(_) => continue,
                Ok(Err(error)) => return Err(error.into()),
                Ok(Ok(0)) => {
                    if !raw && !line.is_empty() {
                        event(&line)?;
                    }
                    // A token-budget observer may request cancellation from
                    // the final unterminated packet, after the last read.
                    if cancel.load(Ordering::Acquire) {
                        return Err(native_cancelled());
                    }
                    break;
                }
                Ok(Ok(n)) => n,
            };
            let remaining = limits.output.saturating_sub(output.len());
            if raw {
                let admitted = count.min(remaining);
                if admitted > 0 {
                    output.extend_from_slice(&chunk[..admitted]);
                    event(&chunk[..admitted])?;
                }
                if admitted < count {
                    return Err(HarnessError {
                        status: 503,
                        code: "output-limit",
                        message: "Native output limit reached.".into(),
                    });
                }
            } else {
                if count > remaining {
                    return Err(HarnessError {
                        status: 503,
                        code: "output-limit",
                        message: "Native output limit reached.".into(),
                    });
                }
                output.extend_from_slice(&chunk[..count]);
                line.extend_from_slice(&chunk[..count]);
                emit_lines(&mut line, limits.line, &mut event)?;
            }
        }
        Ok(())
    }
    .await;
    let status = guardian.finish().await;
    if status == GuardStatus::Unconfirmed {
        writer.abort();
        return Err(cleanup_unconfirmed());
    }
    if let GuardStatus::Native(code) = status {
        observed(&observer, "exited", serde_json::json!({"code":code}))?;
    }
    observed(&observer, "cleanup_confirmed", Value::Null)?;
    let fed = if writer.is_finished() {
        writer.await.ok().and_then(|v| v.err())
    } else {
        writer.abort();
        let _ = writer.await;
        None
    };
    execute?;
    if let Some(error) = fed {
        return Err(error);
    }
    let GuardStatus::Native(code) = status else {
        return Err(status.error().unwrap_or_else(cleanup_unconfirmed));
    };
    Ok(Output {
        code,
        bytes: output,
        seconds: started.elapsed().as_secs_f64(),
    })
}
#[cfg(unix)]
pub(crate) async fn run_captured_until(
    request: crate::capture::Request<'_>,
    mut interrupted: impl FnMut() -> bool + Send,
    mut observed_exit: impl FnMut(crate::capture::NativeExit) + Send,
    mut event: impl FnMut(crate::capture::Stream, &[u8]) -> Result<()> + Send,
) -> Result<crate::capture::Outcome> {
    use crate::capture::{self, Stream};
    validate_command(request.argv, &[])?;
    if interrupted() {
        return Err(capture::cancelled());
    }
    if Instant::now() >= request.deadline {
        return Err(capture::timeout());
    }
    let started = Instant::now();
    let (mut guardian, mut input, mut output, errors, notice) = launch_guardian_options(
        request.argv,
        request.cwd,
        request.environment,
        &[],
        false,
        None,
        CaptureSpawnOptions {
            complete_environment: true,
            capture_stderr: true,
            capture_lifetime: true,
        },
    )
    .await?;
    let Some(mut errors) = errors else {
        drop(input);
        if guardian.finish().await == GuardStatus::Unconfirmed {
            return Err(cleanup_unconfirmed());
        }
        return Err(HarnessError::unavailable(
            "Native captured stderr is unavailable.",
        ));
    };
    let start_error = if interrupted() {
        Some(capture::cancelled())
    } else if Instant::now() >= request.deadline {
        Some(capture::timeout())
    } else {
        None
    };
    let Some(mut notice) = notice else {
        drop(input);
        if guardian.finish().await == GuardStatus::Unconfirmed {
            return Err(cleanup_unconfirmed());
        }
        return Err(HarnessError::unavailable(
            "Native capture exit observation is unavailable.",
        ));
    };
    let mut observation_open = true;
    let mut native: Option<NativeObservation> = None;
    let prompt = request.input.to_vec();
    let admitted = start_error.is_none();
    let mut writer = tokio::spawn(async move {
        if admitted
            && let Err(error) = input.write_all(&prompt).await
            && error.kind() != std::io::ErrorKind::BrokenPipe
        {
            return Err(error.into());
        }
        if let Err(error) = input.shutdown().await
            && error.kind() != std::io::ErrorKind::BrokenPipe
        {
            return Err(error.into());
        }
        Ok::<(), HarnessError>(())
    });
    let mut stdout_open = true;
    let mut stderr_open = true;
    let mut total = 0usize;
    let execute: Result<()> = async {
        if let Some(error) = start_error {
            return Err(error);
        }
        while stdout_open || stderr_open || native.is_none() && guardian.status.is_none() {
            if interrupted() {
                return Err(capture::cancelled());
            }
            if Instant::now() >= request.deadline {
                return Err(capture::timeout());
            }
            let mut stdout_bytes = [0u8; 65536];
            let mut stderr_bytes = [0u8; 65536];
            let read = tokio::select! {
                observed = &mut notice, if observation_open => {
                    observation_open = false;
                    native = observed.ok();
                    if let Some(native) = native {
                        observed_exit(capture::NativeExit {
                            code: native.exit, signal: native.signal, shell_code: native.shell_code,
                        });
                    }
                    continue;
                },
                status = guardian.completed(), if guardian.status.is_none() => {
                    if status == GuardStatus::Unconfirmed { return Err(cleanup_unconfirmed()); }
                    // Drain both bounded pipes after the owner has proved group cleanup.
                    continue;
                },
                result = output.read(&mut stdout_bytes), if stdout_open =>
                    (Stream::Stdout, result.map(|count| stdout_bytes[..count].to_vec())),
                result = errors.read(&mut stderr_bytes), if stderr_open =>
                    (Stream::Stderr, result.map(|count| stderr_bytes[..count].to_vec())),
                _ = tokio::time::sleep(Duration::from_millis(10)) => continue,
            };
            let (stream, bytes) = (read.0, read.1?);
            if bytes.is_empty() {
                match stream {
                    Stream::Stdout => stdout_open = false,
                    Stream::Stderr => stderr_open = false,
                }
                continue;
            }
            total = total
                .checked_add(bytes.len())
                .ok_or_else(capture::overflow)?;
            if total > request.output_limit {
                return Err(capture::overflow());
            }
            event(stream, &bytes)?;
        }
        if interrupted() {
            return Err(capture::cancelled());
        }
        Ok(())
    }
    .await;
    let status = guardian.finish().await;
    // Cleanup may finish before this task polls an already-sent observation.
    // Keep the actual native status even when deadline/cancellation won the
    // final select; never infer it from a cleanup shell code.
    if observation_open && let Ok(observed) = notice.try_recv() {
        native = Some(observed);
        observed_exit(capture::NativeExit {
            code: observed.exit,
            signal: observed.signal,
            shell_code: observed.shell_code,
        });
    }
    if status == GuardStatus::Unconfirmed {
        writer.abort();
        let _ = writer.await;
        return Err(cleanup_unconfirmed());
    }
    let fed = match tokio::time::timeout(Duration::from_secs(1), &mut writer).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(HarnessError::unavailable(
            "Native capture input worker did not finish.",
        )),
        Err(_) => {
            writer.abort();
            let _ = writer.await;
            Err(HarnessError::unavailable(
                "Native capture input worker did not close after process cleanup.",
            ))
        }
    };
    execute?;
    fed?;
    let GuardStatus::Native(code) = status else {
        return Err(status.error().unwrap_or_else(cleanup_unconfirmed));
    };
    let native = native.ok_or_else(|| {
        GuardStatus::Lost
            .error()
            .unwrap_or_else(cleanup_unconfirmed)
    })?;
    if native.shell_code != code {
        return Err(GuardStatus::Protocol
            .error()
            .unwrap_or_else(cleanup_unconfirmed));
    }
    Ok(capture::Outcome {
        exit: native.exit,
        signal: native.signal,
        shell_code: code,
        seconds: started.elapsed().as_secs_f64(),
    })
}

#[cfg(unix)]
fn native_cancelled() -> HarnessError {
    HarnessError {
        status: 409,
        code: "native-cancelled",
        message: "Cancellation requested.".into(),
    }
}
#[cfg(unix)]
fn native_timeout() -> HarnessError {
    HarnessError {
        status: 504,
        code: "native-timeout",
        message: "Native time limit reached.".into(),
    }
}
#[cfg(unix)]
fn cleanup_unconfirmed() -> HarnessError {
    HarnessError {
        status: 503,
        code: "cleanup-unconfirmed",
        message: "Native cleanup could not be confirmed; stop admission before another run.".into(),
    }
}
#[cfg(unix)]
fn validate_command(argv: &[String], remove: &[String]) -> Result<()> {
    if argv.is_empty()
        || argv.iter().any(|s| s.contains('\0'))
        || remove
            .iter()
            .any(|s| s.is_empty() || s.contains(['\0', '=']))
    {
        return Err(HarnessError::invalid(
            "Invalid native command or environment removal.",
        ));
    }
    Ok(())
}
#[cfg(unix)]
const EXIT_PACKET_LEN: usize = 32;
#[cfg(unix)]
fn exit_packet(nonce: &[u8; 16], status: std::process::ExitStatus) -> [u8; EXIT_PACKET_LEN] {
    use std::os::unix::process::ExitStatusExt;
    let mut packet = [0u8; EXIT_PACKET_LEN];
    packet[..4].copy_from_slice(b"IG01");
    packet[4] = 1; // Observed native wait status, not a cleanup acknowledgement.
    packet[8..24].copy_from_slice(nonce);
    packet[24..28].copy_from_slice(&status.into_raw().to_be_bytes());
    packet[28..32].copy_from_slice(&(EXIT_PACKET_LEN as u32).to_be_bytes());
    packet
}
#[cfg(unix)]
fn parse_exit_packet(packet: &[u8], nonce: &[u8; 16]) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    if packet.len() != EXIT_PACKET_LEN
        || &packet[..4] != b"IG01"
        || packet[4] != 1
        || packet[5..8] != [0, 0, 0]
        || &packet[8..24] != nonce
        || packet[28..32] != (EXIT_PACKET_LEN as u32).to_be_bytes()
    {
        return None;
    }
    let raw = i32::from_be_bytes(packet[24..28].try_into().ok()?);
    if !(libc::WIFEXITED(raw) && raw & !0xff00 == 0 || libc::WIFSIGNALED(raw) && raw & !0xff == 0) {
        return None;
    }
    let status = std::process::ExitStatus::from_raw(raw);
    Some(
        status
            .code()
            .unwrap_or_else(|| 128 + status.signal().unwrap_or(1)),
    )
}
#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GuardStatus {
    Native(i32),
    Lost,
    Protocol,
    Unconfirmed,
}
#[cfg(unix)]
impl GuardStatus {
    fn error(self) -> Option<HarnessError> {
        match self {
            Self::Native(_) => None,
            Self::Unconfirmed => Some(cleanup_unconfirmed()),
            Self::Lost | Self::Protocol => Some(HarnessError {
                status: 503,
                code: "native-guardian-exited",
                message: "The native guardian exited without a valid native exit receipt.".into(),
            }),
        }
    }
}
#[cfg(unix)]
struct Guardian {
    pid: u32,
    stop: Option<std::sync::mpsc::Sender<()>>,
    done: tokio::sync::oneshot::Receiver<GuardStatus>,
    status: Option<GuardStatus>,
}
#[cfg(unix)]
impl Guardian {
    async fn completed(&mut self) -> GuardStatus {
        if let Some(status) = self.status {
            return status;
        }
        let status = (&mut self.done).await.unwrap_or(GuardStatus::Unconfirmed);
        self.status = Some(status);
        status
    }
    async fn finish(&mut self) -> GuardStatus {
        self.stop.take();
        self.completed().await
    }
}
#[cfg(unix)]
#[derive(Clone, Copy)]
struct NativeObservation {
    exit: Option<i32>,
    signal: Option<i32>,
    shell_code: i32,
}
#[cfg(unix)]
struct GuardResources {
    child: std::process::Child,
    watchdog: Option<std::os::fd::OwnedFd>,
    receipt: std::os::fd::OwnedFd,
    nonce: [u8; 16],
    capture_lifetime: bool,
    native_observation: Option<tokio::sync::oneshot::Sender<NativeObservation>>,
    // The independent owner retains exclusion even if its async caller/runtime
    // disappears or cleanup proof takes longer than the caller's budget.
    _runner_lock: Option<Arc<std::fs::File>>,
}
#[cfg(unix)]
async fn launch_guardian(
    argv: &[String],
    cwd: &Path,
    environment: &BTreeMap<String, String>,
    remove: &[String],
    raw: bool,
    runner_lock: Option<Arc<std::fs::File>>,
) -> Result<(
    Guardian,
    tokio::process::ChildStdin,
    tokio::process::ChildStdout,
)> {
    let (guardian, stdin, stdout, _, _) = launch_guardian_options(
        argv,
        cwd,
        environment,
        remove,
        raw,
        runner_lock,
        CaptureSpawnOptions {
            complete_environment: false,
            capture_stderr: false,
            capture_lifetime: false,
        },
    )
    .await?;
    Ok((guardian, stdin, stdout))
}
#[cfg(unix)]
struct CaptureSpawnOptions {
    complete_environment: bool,
    capture_stderr: bool,
    capture_lifetime: bool,
}
#[cfg(unix)]
type CapturedPipes = (
    Guardian,
    tokio::process::ChildStdin,
    tokio::process::ChildStdout,
    Option<tokio::process::ChildStderr>,
    Option<tokio::sync::oneshot::Receiver<NativeObservation>>,
);
#[cfg(unix)]
async fn launch_guardian_options(
    argv: &[String],
    cwd: &Path,
    environment: &BTreeMap<String, String>,
    remove: &[String],
    raw: bool,
    runner_lock: Option<Arc<std::fs::File>>,
    options: CaptureSpawnOptions,
) -> Result<CapturedPipes> {
    use std::os::{fd::AsRawFd, unix::process::CommandExt};
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return Err(HarnessError::unavailable(
        "Native group custody is unavailable on this platform.",
    ));
    let mut disposition = std::mem::MaybeUninit::<libc::sigaction>::uninit();
    if unsafe { libc::sigaction(libc::SIGCHLD, std::ptr::null(), disposition.as_mut_ptr()) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let disposition = unsafe { disposition.assume_init() };
    if disposition.sa_sigaction == libc::SIG_IGN || disposition.sa_flags & libc::SA_NOCLDWAIT != 0 {
        return Err(HarnessError::unavailable(
            "Native custody requires retained child wait ownership.",
        ));
    }
    let watch = Watchdog::new()?;
    let receipt = Watchdog::new()?;
    let nonce = *uuid::Uuid::new_v4().as_bytes();
    let mut command = std::process::Command::new(guardian()?);
    command.arg("internal-guard");
    watch.configure(&mut command)?;
    let receipt_fd = receipt.write.as_raw_fd();
    command
        .arg("--receipt-fd")
        .arg(receipt_fd.to_string())
        .arg("--receipt-nonce")
        .arg(uuid::Uuid::from_bytes(nonce).to_string());
    unsafe {
        command.pre_exec(move || {
            let flags = libc::fcntl(receipt_fd, libc::F_GETFD);
            if flags < 0 || libc::fcntl(receipt_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    if let Some(file) = &runner_lock {
        let fd = file.as_raw_fd();
        command.arg("--runner-lock").arg(fd.to_string());
        unsafe {
            command.pre_exec(move || {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    if raw {
        command.arg("--merge-stderr");
    }
    if options.capture_lifetime {
        command.arg("--capture-stdio");
    }
    if options.complete_environment {
        command.env_clear();
    }
    command
        .arg("--")
        .args(argv)
        .current_dir(cwd)
        .envs(environment)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(if options.capture_stderr {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        });
    for name in remove {
        command.env_remove(name);
    }
    let flags = unsafe { libc::fcntl(receipt.read.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe {
            libc::fcntl(
                receipt.read.as_raw_fd(),
                libc::F_SETFL,
                flags | libc::O_NONBLOCK,
            )
        } < 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let (start, receive) = std::sync::mpsc::sync_channel::<GuardResources>(1);
    let (stop, stopping) = std::sync::mpsc::channel();
    let (notify, done) = tokio::sync::oneshot::channel();
    // Create the independent owner before creating any OS child. There is no
    // fallible thread-spawn path that can drop an already-created child/lock.
    let _owner = std::thread::Builder::new()
        .name("infra-native-custody".into())
        .spawn(move || {
            if let Ok(resources) = receive.recv() {
                own_guardian(resources, stopping, notify);
            }
        })?;
    let mut child = command.spawn()?;
    let pid = child.id();
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (notice, observation) = if options.capture_lifetime {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        (Some(sender), Some(receiver))
    } else {
        (None, None)
    };
    let resources = GuardResources {
        child,
        watchdog: Some(watch.retain_parent()),
        receipt: receipt.retain_reader(),
        nonce,
        capture_lifetime: options.capture_lifetime,
        native_observation: notice,
        _runner_lock: runner_lock,
    };
    if let Err(error) = start.send(resources) {
        // The owner cannot normally exit before receiving its one child. If it
        // does, keep this actual child pinned while issuing the final signal,
        // then quarantine both the unreaped pin and exclusion instead of
        // allowing a dropped std::Child to release custody.
        let resources = error.0;
        let _ = signal_group(resources.child.id() as i32, libc::SIGKILL);
        std::mem::forget(resources);
        return Err(cleanup_unconfirmed());
    }
    drop(start);
    let mut guardian = Guardian {
        pid,
        stop: Some(stop),
        done,
        status: None,
    };
    let pipes = match (stdin, stdout, stderr) {
        (Some(stdin), Some(stdout), stderr) if !options.capture_stderr || stderr.is_some() => {
            tokio::process::ChildStdin::from_std(stdin).and_then(|stdin| {
                tokio::process::ChildStdout::from_std(stdout).and_then(|stdout| {
                    stderr
                        .map(tokio::process::ChildStderr::from_std)
                        .transpose()
                        .map(|stderr| (stdin, stdout, stderr))
                })
            })
        }
        _ => Err(std::io::Error::other("Native pipes are unavailable.")),
    };
    match pipes {
        Ok((stdin, stdout, stderr)) => Ok((guardian, stdin, stdout, stderr, observation)),
        Err(error) => {
            let status = guardian.finish().await;
            if status == GuardStatus::Unconfirmed {
                Err(cleanup_unconfirmed())
            } else {
                Err(error.into())
            }
        }
    }
}
#[cfg(unix)]
fn signal_group(group: i32, signal: i32) -> std::io::Result<()> {
    if group <= 2 || group == unsafe { libc::getpgrp() } {
        return Err(std::io::Error::other("Invalid owned native group."));
    }
    if unsafe { libc::kill(-group, signal) } < 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error);
        }
    }
    Ok(())
}
#[cfg(target_os = "linux")]
fn group_live(group: i32) -> std::io::Result<bool> {
    let mut anchor = false;
    let mut live = false;
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        // Do not require access to unrelated users' private /proc records.
        let pgrp = unsafe { libc::getpgid(pid) };
        if pgrp < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                continue;
            }
            return Err(error);
        }
        if pgrp != group {
            continue;
        }
        let stat = match std::fs::read_to_string(entry.path().join("stat")) {
            Ok(stat) => stat,
            Err(error) if matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ESRCH)) => {
                continue;
            }
            Err(error) => return Err(error),
        };
        let fields = stat
            .rsplit_once(") ")
            .ok_or_else(|| std::io::Error::other("Invalid process identity."))?
            .1;
        let mut fields = fields.split_whitespace();
        let state = fields
            .next()
            .ok_or_else(|| std::io::Error::other("Missing process state."))?;
        fields.next();
        let pgrp = fields
            .next()
            .and_then(|field| field.parse::<i32>().ok())
            .ok_or_else(|| std::io::Error::other("Missing process group."))?;
        if pgrp != group {
            continue;
        }
        anchor |= pid == group;
        live |= !matches!(state, "Z" | "X");
    }
    if !anchor {
        return Err(std::io::Error::other(
            "Owned guardian anchor is unavailable.",
        ));
    }
    Ok(live)
}
#[cfg(target_os = "macos")]
fn group_live(group: i32) -> std::io::Result<bool> {
    let mut pids = vec![0i32; 128];
    let count = loop {
        let count = unsafe {
            libc::proc_listpgrppids(
                group,
                pids.as_mut_ptr().cast(),
                std::mem::size_of_val(pids.as_slice()) as i32,
            )
        };
        // This libproc wrapper returns a PID count, unlike proc_listpids.
        if count <= 0 {
            return Err(std::io::Error::last_os_error());
        }
        let count = count as usize;
        if count < pids.len() {
            break count;
        }
        if pids.len() >= 1_048_576 {
            return Err(std::io::Error::other("Native process group is too large."));
        }
        pids.resize(pids.len() * 2, 0);
    };
    let mut anchor = false;
    let mut live = false;
    for pid in pids.into_iter().take(count).filter(|pid| *pid > 0) {
        let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
        // A nonzero argument includes zombies, especially our unreaped anchor.
        let bytes = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                1,
                info.as_mut_ptr().cast(),
                size,
            )
        };
        if bytes != size {
            let error = std::io::Error::last_os_error();
            if pid != group && matches!(error.raw_os_error(), Some(libc::ESRCH | libc::ENOENT)) {
                continue;
            }
            return Err(error);
        }
        let info = unsafe { info.assume_init() };
        if info.pbi_pid != pid as u32 || info.pbi_pgid != group as u32 {
            continue;
        }
        anchor |= pid == group;
        live |= info.pbi_status != libc::SZOMB;
    }
    if !anchor {
        return Err(std::io::Error::other(
            "Owned guardian anchor is unavailable.",
        ));
    }
    Ok(live)
}
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn group_live(_: i32) -> std::io::Result<bool> {
    Err(std::io::Error::other(
        "Native process group proof is unavailable.",
    ))
}
#[cfg(unix)]
fn own_guardian(
    mut resources: GuardResources,
    stopping: std::sync::mpsc::Receiver<()>,
    notify: tokio::sync::oneshot::Sender<GuardStatus>,
) {
    use std::os::fd::AsRawFd;
    let group = resources.child.id() as i32;
    let mut notify = Some(notify);
    let mut packet = Vec::new();
    let mut eof = false;
    let mut protocol = false;
    let mut cleanup = None;
    let mut killed = false;
    let mut quiet = 0u8;
    let mut observed_exit = false;
    let mut lost_capture_guardian = false;
    loop {
        if !eof {
            let mut bytes = [0u8; EXIT_PACKET_LEN + 1];
            let count = unsafe {
                libc::read(
                    resources.receipt.as_raw_fd(),
                    bytes.as_mut_ptr().cast(),
                    bytes.len(),
                )
            };
            if count > 0 {
                if packet.len() + count as usize > EXIT_PACKET_LEN {
                    protocol = true;
                } else {
                    packet.extend_from_slice(&bytes[..count as usize]);
                }
            } else if count == 0 {
                eof = true;
            } else {
                let error = std::io::Error::last_os_error();
                if !matches!(error.raw_os_error(), Some(libc::EAGAIN | libc::EINTR)) {
                    protocol = true;
                    eof = true;
                }
            }
        }
        let requested = !matches!(
            stopping.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        );
        if resources.capture_lifetime
            && !observed_exit
            && packet.len() == EXIT_PACKET_LEN
            && !protocol
        {
            if let Some(shell_code) = parse_exit_packet(&packet, &resources.nonce) {
                use std::os::unix::process::ExitStatusExt;
                let raw =
                    i32::from_be_bytes(packet[24..28].try_into().expect("validated exit packet"));
                let native = std::process::ExitStatus::from_raw(raw);
                if let Some(sender) = resources.native_observation.take() {
                    let _ = sender.send(NativeObservation {
                        exit: native.code(),
                        signal: native.signal(),
                        shell_code,
                    });
                }
                observed_exit = true;
            } else {
                protocol = true;
            }
        }
        if resources.capture_lifetime && eof && cleanup.is_none() && !requested {
            // Capture guardians stay alive after the native exit observation;
            // unexpected guardian loss must never be adopted as a normal exit.
            lost_capture_guardian = true;
        }
        if cleanup.is_none()
            && (requested
                || eof
                || protocol
                || !resources.capture_lifetime && packet.len() == EXIT_PACKET_LEN)
        {
            resources.watchdog.take();
            cleanup = Some(Instant::now());
            // This thread exclusively owns the unreaped direct child. Its PID
            // pins this PGID through every signal, including error recovery.
            let _ = signal_group(group, libc::SIGCONT);
            let _ = signal_group(group, libc::SIGTERM);
        }
        if let Some(started) = cleanup {
            if started.elapsed() >= Duration::from_millis(75) {
                let _ = signal_group(group, libc::SIGKILL);
                killed = true;
            }
            if killed && eof && matches!(group_live(group), Ok(false)) {
                quiet += 1;
            } else {
                quiet = 0;
            }
            if quiet >= 2 {
                // No live member can now fork. Only after this proof may wait
                // release the anchor. There are no group signals after wait.
                if resources.child.wait().is_err() {
                    if let Some(notify) = notify.take() {
                        let _ = notify.send(GuardStatus::Unconfirmed);
                    }
                    std::mem::forget(resources);
                    return;
                }
                let status = if protocol {
                    GuardStatus::Protocol
                } else if lost_capture_guardian {
                    GuardStatus::Lost
                } else {
                    parse_exit_packet(&packet, &resources.nonce)
                        .map(GuardStatus::Native)
                        .unwrap_or(GuardStatus::Lost)
                };
                drop(resources);
                if let Some(notify) = notify.take() {
                    let _ = notify.send(status);
                }
                return;
            }
            if started.elapsed() >= Duration::from_secs(3)
                && let Some(notify) = notify.take()
            {
                let _ = notify.send(GuardStatus::Unconfirmed);
                // Keep both actual child pin and runner exclusion. Continue
                // owned cleanup outside Tokio; never reap before its proof.
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[cfg(unix)]
struct Watchdog {
    read: std::os::fd::OwnedFd,
    write: std::os::fd::OwnedFd,
}
#[cfg(unix)]
impl Watchdog {
    fn new() -> Result<Self> {
        use std::os::fd::FromRawFd;
        let mut fds = [-1; 2];
        #[cfg(target_os = "linux")]
        let result = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
        #[cfg(not(target_os = "linux"))]
        let result = unsafe { libc::pipe(fds.as_mut_ptr()) };
        if result != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        for fd in fds {
            if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
                unsafe {
                    libc::close(fds[0]);
                    libc::close(fds[1]);
                }
                return Err(std::io::Error::last_os_error().into());
            }
        }
        Ok(Self {
            read: unsafe { std::os::fd::OwnedFd::from_raw_fd(fds[0]) },
            write: unsafe { std::os::fd::OwnedFd::from_raw_fd(fds[1]) },
        })
    }
    fn configure(&self, command: &mut std::process::Command) -> Result<()> {
        use std::os::{fd::AsRawFd, unix::process::CommandExt};
        let read = self.read.as_raw_fd();
        command.arg("--watchdog-fd").arg(read.to_string());
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                let flags = libc::fcntl(read, libc::F_GETFD);
                if flags < 0 || libc::fcntl(read, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok(())
    }
    fn retain_parent(self) -> std::os::fd::OwnedFd {
        self.write
    }
    fn retain_reader(self) -> std::os::fd::OwnedFd {
        self.read
    }
}
#[cfg(unix)]
fn guard_descriptor_custody(watch: i32, receipt: i32, runner_lock: Option<i32>) -> Result<()> {
    fn validate(fd: i32, kind: libc::mode_t) -> Result<()> {
        let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe { libc::fstat(fd, info.as_mut_ptr()) } < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if unsafe { info.assume_init() }.st_mode & libc::S_IFMT != kind {
            return Err(HarnessError::invalid(
                "Invalid native guard descriptor kind.",
            ));
        }
        Ok(())
    }
    validate(watch, libc::S_IFIFO)?;
    if receipt < 3 || receipt == watch {
        return Err(HarnessError::invalid("Invalid native receipt descriptor."));
    }
    validate(receipt, libc::S_IFIFO)?;
    if let Some(fd) = runner_lock {
        if fd < 3 || fd == watch || fd == receipt {
            return Err(HarnessError::invalid(
                "Invalid native runner-lock descriptor.",
            ));
        }
        validate(fd, libc::S_IFREG)?;
    }
    // The internal guardian enters directly, before an async runtime is started.
    // Enumerate and change flags before fork; pre_exec performs no allocation.
    #[cfg(target_os = "linux")]
    let entries = std::fs::read_dir("/proc/self/fd").or_else(|_| std::fs::read_dir("/dev/fd"))?;
    #[cfg(not(target_os = "linux"))]
    let entries = std::fs::read_dir("/dev/fd")?;
    let mut descriptors = entries
        .map(|entry| {
            entry?
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<i32>().ok())
                .ok_or_else(|| {
                    HarnessError::unavailable("Cannot enumerate native guard descriptors.")
                })
        })
        .collect::<Result<Vec<_>>>()?;
    // ReadDir has closed its own descriptor. Explicit custody descriptors must
    // remain valid even if the platform listing did not include them.
    descriptors.push(watch);
    descriptors.push(receipt);
    if let Some(fd) = runner_lock {
        descriptors.push(fd);
    }
    for fd in descriptors.into_iter().filter(|fd| *fd > 2) {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EBADF)
                && fd != watch
                && fd != receipt
                && Some(fd) != runner_lock
            {
                continue;
            }
            return Err(error.into());
        }
        let flags = if Some(fd) == runner_lock {
            flags & !libc::FD_CLOEXEC
        } else {
            flags | libc::FD_CLOEXEC
        };
        if unsafe { libc::fcntl(fd, libc::F_SETFD, flags) } < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(())
}
#[cfg(unix)]
static GUARD_TERMINATING: AtomicBool = AtomicBool::new(false);
#[cfg(unix)]
extern "C" fn guard_signal(_: i32) {
    GUARD_TERMINATING.store(true, Ordering::Release);
}
#[cfg(unix)]
fn guard_end_group(group: i32) -> ! {
    // The guardian deliberately stays in its own anchored group. Its handler
    // survives TERM long enough to issue the final KILL even on parent EOF.
    let _ = signal_group_from_guard(group, libc::SIGTERM);
    std::thread::sleep(Duration::from_millis(50));
    loop {
        let _ = signal_group_from_guard(group, libc::SIGKILL);
        std::thread::sleep(Duration::from_millis(50));
    }
}
#[cfg(unix)]
fn signal_group_from_guard(group: i32, signal: i32) -> std::io::Result<()> {
    if group != unsafe { libc::getpid() } || group != unsafe { libc::getpgrp() } {
        return Err(std::io::Error::other(
            "Guardian is not its owned group anchor.",
        ));
    }
    if unsafe { libc::kill(-group, signal) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}
#[cfg(unix)]
fn guard_publish_exit(
    child: &mut std::process::Child,
    receipt: &mut std::fs::File,
    nonce: &[u8; 16],
    sent: &mut bool,
) -> std::io::Result<()> {
    use std::io::Write;
    if !*sent && let Some(status) = child.try_wait()? {
        receipt.write_all(&exit_packet(nonce, status))?;
        receipt.flush()?;
        *sent = true;
    }
    Ok(())
}
#[cfg(unix)]
fn guard_capture_cleanup(
    group: i32,
    capture_stdio: bool,
    child: &mut std::process::Child,
    receipt: &mut std::fs::File,
    nonce: &[u8; 16],
    sent: &mut bool,
) -> ! {
    // A captured native process may already have exited while its guardian
    // was stopped or before the next poll. Publish that actual wait status
    // before cleanup kills the guardian; the group remains pinned by the
    // parent's unreaped guardian throughout all signals.
    if capture_stdio {
        let _ = guard_publish_exit(child, receipt, nonce, sent);
    }
    guard_end_group(group)
}
#[cfg(unix)]
pub fn guard_entry(arguments: &[String]) -> Result<i32> {
    if arguments.first().map(String::as_str) == Some("--capture-anchor") {
        return experiment::anchor_entry(&arguments[1..]);
    }
    if arguments.first().map(String::as_str) == Some("--capture-observer") {
        return experiment::observer_entry(&arguments[1..]);
    }
    use std::os::{fd::FromRawFd, unix::process::CommandExt};
    let separator = arguments
        .iter()
        .position(|s| s == "--")
        .ok_or_else(|| HarnessError::invalid("Missing native watchdog command."))?;
    let mut fd = None;
    let mut receipt = None;
    let mut nonce = None;
    let mut runner_lock = None;
    let mut merge = false;
    let mut capture_stdio = false;
    let mut at = 0usize;
    while at < separator {
        match arguments[at].as_str() {
            "--watchdog-fd" | "--receipt-fd" | "--runner-lock" => {
                let option = arguments[at].as_str();
                at += 1;
                let value = arguments
                    .get(at)
                    .filter(|_| at < separator)
                    .and_then(|s| s.parse::<i32>().ok())
                    .ok_or_else(|| HarnessError::invalid("Invalid guard descriptor."))?;
                match option {
                    "--watchdog-fd" if fd.is_none() => fd = Some(value),
                    "--receipt-fd" if receipt.is_none() => receipt = Some(value),
                    "--runner-lock" if runner_lock.is_none() => runner_lock = Some(value),
                    _ => return Err(HarnessError::invalid("Duplicate guard descriptor.")),
                }
            }
            "--receipt-nonce" => {
                at += 1;
                let value = arguments
                    .get(at)
                    .filter(|_| at < separator)
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    .ok_or_else(|| HarnessError::invalid("Invalid guard receipt nonce."))?;
                if nonce.replace(*value.as_bytes()).is_some() {
                    return Err(HarnessError::invalid("Duplicate guard receipt nonce."));
                }
            }
            "--merge-stderr" if !merge => merge = true,
            "--capture-stdio" if !capture_stdio => capture_stdio = true,
            _ => return Err(HarnessError::invalid("Unknown watchdog option.")),
        }
        at += 1;
    }
    let fd = fd
        .filter(|fd| *fd >= 3)
        .ok_or_else(|| HarnessError::invalid("Missing native watchdog."))?;
    let receipt = receipt.ok_or_else(|| HarnessError::invalid("Missing native receipt pipe."))?;
    let nonce = nonce.ok_or_else(|| HarnessError::invalid("Missing native receipt nonce."))?;
    if arguments.len() <= separator + 1 {
        return Err(HarnessError::invalid("Missing native command."));
    }
    let group = unsafe { libc::getpid() };
    if group <= 2 || unsafe { libc::getpgrp() } != group {
        return Err(HarnessError::invalid(
            "Native guardian must own its process group.",
        ));
    }
    guard_descriptor_custody(fd, receipt, runner_lock)?;
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        if unsafe { libc::signal(signal, guard_signal as *const () as libc::sighandler_t) }
            == libc::SIG_ERR
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    // Parent death may close the receipt reader just before a native exit.
    // Handle EPIPE by cleaning the group rather than dying before cleanup.
    if unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) } == libc::SIG_ERR {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut empty = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
    if unsafe { libc::sigemptyset(empty.as_mut_ptr()) } < 0
        || unsafe { libc::sigprocmask(libc::SIG_SETMASK, empty.as_ptr(), std::ptr::null_mut()) } < 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut receipt = unsafe { std::fs::File::from_raw_fd(receipt) };
    let mut command = std::process::Command::new(&arguments[separator + 1]);
    command.args(&arguments[separator + 2..]);
    unsafe {
        command.pre_exec(move || {
            // The native leaf shares the guardian PGID but inherits normal
            // dispositions and an unblocked mask, never the guardian handler.
            for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGPIPE] {
                if libc::signal(signal, libc::SIG_DFL) == libc::SIG_ERR {
                    return Err(std::io::Error::last_os_error());
                }
            }
            let mut empty = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
            if libc::sigemptyset(empty.as_mut_ptr()) < 0
                || libc::sigprocmask(libc::SIG_SETMASK, empty.as_ptr(), std::ptr::null_mut()) < 0
            {
                return Err(std::io::Error::last_os_error());
            }
            if merge && libc::dup2(1, 2) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    if capture_stdio {
        // Only native processes now own the captured streams. The guardian
        // keeps its private watchdog/receipt and optional runner descriptors.
        for fd in [0, 1, 2] {
            unsafe {
                libc::close(fd);
            }
        }
    }
    let mut sent = false;
    loop {
        if GUARD_TERMINATING.load(Ordering::Acquire) {
            guard_capture_cleanup(
                group,
                capture_stdio,
                &mut child,
                &mut receipt,
                &nonce,
                &mut sent,
            );
        }
        if guard_publish_exit(&mut child, &mut receipt, &nonce, &mut sent).is_err() {
            guard_capture_cleanup(
                group,
                capture_stdio,
                &mut child,
                &mut receipt,
                &nonce,
                &mut sent,
            );
        }
        let mut poll = libc::pollfd {
            fd,
            events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
            revents: 0,
        };
        let polled = unsafe { libc::poll(&mut poll, 1, 20) };
        if polled < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            guard_capture_cleanup(
                group,
                capture_stdio,
                &mut child,
                &mut receipt,
                &nonce,
                &mut sent,
            );
        }
        if poll.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
            guard_capture_cleanup(
                group,
                capture_stdio,
                &mut child,
                &mut receipt,
                &nonce,
                &mut sent,
            );
        }
        if poll.revents & libc::POLLIN != 0 {
            let mut byte = [0u8; 64];
            if unsafe { libc::read(fd, byte.as_mut_ptr().cast(), byte.len()) } <= 0 {
                guard_capture_cleanup(
                    group,
                    capture_stdio,
                    &mut child,
                    &mut receipt,
                    &nonce,
                    &mut sent,
                );
            }
        }
    }
}
#[cfg(windows)]
#[path = "process_windows.rs"]
mod windows;
#[cfg(windows)]
pub(crate) use windows::CaptureChild;
#[cfg(windows)]
pub use windows::{
    GuardedChild, NativeSession, converse, converse_environment, guard_entry, is_stopping, run,
    run_environment, run_monitored, run_monitored_until, run_raw_with_lock, run_with_lock,
};

static GUARDIAN: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Names the engine whose `internal-guard` command guards child processes.
/// Without it the current executable guards them, as the engine does for
/// itself. A process that embeds this library and cannot serve that command,
/// such as a test binary, names an engine before it starts any child; the
/// first engine named stays for the life of the process.
pub fn use_guardian(engine: std::path::PathBuf) -> Result<()> {
    if !engine.is_absolute() {
        return Err(crate::HarnessError::invalid(
            "The guardian engine path must be absolute.",
        ));
    }
    if *GUARDIAN.get_or_init(|| engine.clone()) != engine {
        return Err(crate::HarnessError::conflict(
            "Another guardian engine is already in use.",
        ));
    }
    Ok(())
}
fn guardian() -> Result<std::path::PathBuf> {
    match GUARDIAN.get() {
        Some(engine) => Ok(engine.clone()),
        None => Ok(std::env::current_exe()?),
    }
}

#[cfg(unix)]
pub async fn run(
    argv: &[String],
    cwd: &Path,
    environment: &BTreeMap<String, String>,
    input: &[u8],
    limits: Limits,
    cancel: Arc<AtomicBool>,
    event: impl FnMut(&[u8]) -> Result<()> + Send,
) -> Result<Output> {
    run_with_lock(argv, cwd, environment, input, limits, cancel, None, event).await
}

pub enum Control {
    Continue,
    Write(Vec<Value>),
    Done(Value),
}
#[cfg(unix)]
#[expect(
    clippy::too_many_arguments,
    reason = "The stable native IPC boundary keeps runner custody and response handling explicit."
)]
pub async fn converse(
    argv: &[String],
    cwd: &Path,
    environment: &BTreeMap<String, String>,
    first: &[Value],
    limits: Limits,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<std::fs::File>>,
    answer: impl FnMut(Value) -> Result<Control> + Send,
) -> Result<Value> {
    converse_environment(
        argv,
        cwd,
        environment,
        &[],
        first,
        limits,
        cancel,
        runner_lock,
        answer,
    )
    .await
}
#[cfg(unix)]
#[expect(
    clippy::too_many_arguments,
    reason = "The stable native IPC boundary keeps scoped environment removal and runner custody explicit."
)]
pub async fn converse_environment(
    argv: &[String],
    cwd: &Path,
    environment: &BTreeMap<String, String>,
    remove: &[String],
    first: &[Value],
    limits: Limits,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<std::fs::File>>,
    mut answer: impl FnMut(Value) -> Result<Control> + Send,
) -> Result<Value> {
    validate_command(argv, remove)?;
    if cancel.load(Ordering::Acquire) {
        return Err(native_cancelled());
    }
    let (mut guardian, mut input, output) =
        launch_guardian(argv, cwd, environment, remove, false, runner_lock).await?;
    let mut output = BufReader::new(output);
    let deadline = Instant::now() + Duration::from_secs(limits.seconds.unwrap_or(10));
    let result = async {
        let mut sent = 0usize;
        for value in first {
            let mut bytes = serde_json::to_vec(value)?;
            bytes.push(b'\n');
            sent = sent.saturating_add(bytes.len());
            if sent > 65536 {
                return Err(HarnessError::invalid("Native protocol input limit reached."));
            }
            protocol_write(&mut guardian, &mut input, &bytes, deadline, cancel.as_ref()).await?;
        }
        let mut total = 0usize;
        let mut line = Vec::new();
        loop {
            if cancel.load(Ordering::Acquire) { return Err(native_cancelled()); }
            if Instant::now() > deadline {
                return Err(HarnessError::unavailable("The CLI did not answer in time."));
            }
            let mut chunk = [0u8; 65536];
            let read = tokio::select! {
                status = guardian.completed(), if guardian.status.is_none() => {
                    if let Some(error) = status.error() { return Err(error); }
                    continue;
                },
                read = tokio::time::timeout(Duration::from_millis(100), output.read(&mut chunk)) => read,
            };
            let n = match read {
                Err(_) => continue,
                Ok(result) => result?,
            };
            if n == 0 {
                return Err(HarnessError::unavailable("The CLI exited without answering."));
            }
            total = total.saturating_add(n);
            if total > limits.output {
                return Err(HarnessError::unavailable("The CLI answered with more output than expected."));
            }
            line.extend_from_slice(&chunk[..n]);
            while let Some(end) = line.iter().position(|b| *b == b'\n') {
                if end > limits.line {
                    return Err(HarnessError::unavailable("Native protocol line limit reached."));
                }
                let bytes = line.drain(..=end).collect::<Vec<_>>();
                let Ok(message) = crate::strict_json::parse(&bytes, limits.line) else { continue; };
                if !message.is_object() { continue; }
                let control = answer(message)?;
                if cancel.load(Ordering::Acquire) { return Err(native_cancelled()); }
                match control {
                    Control::Continue => {}
                    Control::Done(value) => return Ok(value),
                    Control::Write(messages) => {
                        for value in messages {
                            let mut bytes = serde_json::to_vec(&value)?;
                            bytes.push(b'\n');
                            sent = sent.saturating_add(bytes.len());
                            if sent > 65536 {
                                return Err(HarnessError::invalid("Native protocol input limit reached."));
                            }
                            protocol_write(&mut guardian, &mut input, &bytes, deadline, cancel.as_ref()).await?;
                        }
                    }
                }
            }
            if line.len() > limits.line {
                return Err(HarnessError::unavailable("Native protocol line limit reached."));
            }
        }
    }.await;
    let _ = tokio::time::timeout(Duration::from_secs(1), input.shutdown()).await;
    drop(input);
    let status = guardian.finish().await;
    if status == GuardStatus::Unconfirmed {
        return Err(cleanup_unconfirmed());
    }
    if status == GuardStatus::Protocol {
        return Err(status.error().unwrap_or_else(cleanup_unconfirmed));
    }
    // A completed interactive exchange intentionally stops a still-running
    // server. Its response is valid after group cleanup even without a native
    // exit packet; unexpected guardian loss during the exchange is an error.
    result
}
#[cfg(unix)]
async fn protocol_write(
    guardian: &mut Guardian,
    input: &mut tokio::process::ChildStdin,
    bytes: &[u8],
    deadline: Instant,
    cancel: &AtomicBool,
) -> Result<()> {
    if let Some(status) = guardian.status {
        return Err(status.error().unwrap_or_else(|| {
            HarnessError::unavailable("The CLI exited before accepting protocol input.")
        }));
    }
    // Keep one write future alive across cancellation polls. Recreating
    // write_all after a partial write could duplicate a protocol prefix.
    let writing = tokio::time::timeout_at(
        tokio::time::Instant::from_std(deadline),
        input.write_all(bytes),
    );
    tokio::pin!(writing);
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(native_cancelled());
        }
        tokio::select! {
            status = guardian.completed() => return Err(status.error().unwrap_or_else(|| {
                HarnessError::unavailable("The CLI exited before accepting protocol input.")
            })),
            result = &mut writing => {
                result.map_err(|_| HarnessError::unavailable("Native protocol write timed out."))??;
                if cancel.load(Ordering::Acquire) {
                    return Err(native_cancelled());
                }
                return Ok(());
            },
            _ = tokio::time::sleep(Duration::from_millis(100)) => {},
        }
    }
}

#[cfg(all(test, unix))]
mod receipt_tests {
    use super::{EXIT_PACKET_LEN, exit_packet, parse_exit_packet};
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn native_exit_125_is_a_valid_status_packet() {
        let nonce = [41; 16];
        let packet = exit_packet(&nonce, std::process::ExitStatus::from_raw(125 << 8));
        assert_eq!(parse_exit_packet(&packet, &nonce), Some(125));
        let signal = exit_packet(&nonce, std::process::ExitStatus::from_raw(libc::SIGKILL));
        assert_eq!(
            parse_exit_packet(&signal, &nonce),
            Some(128 + libc::SIGKILL)
        );
    }

    #[test]
    fn exit_packet_rejects_missing_partial_and_trailing_bytes() {
        let nonce = [42; 16];
        let packet = exit_packet(&nonce, std::process::ExitStatus::from_raw(0));
        for length in 0..EXIT_PACKET_LEN {
            assert_eq!(parse_exit_packet(&packet[..length], &nonce), None);
        }
        let mut trailing = packet.to_vec();
        trailing.push(0);
        assert_eq!(parse_exit_packet(&trailing, &nonce), None);
    }

    #[test]
    fn exit_packet_binds_nonce_magic_tag_reserved_and_length() {
        let nonce = [43; 16];
        let packet = exit_packet(&nonce, std::process::ExitStatus::from_raw(0));
        assert_eq!(parse_exit_packet(&packet, &[44; 16]), None);
        for offset in [0, 4, 5, 8, 28, 31] {
            let mut changed = packet;
            changed[offset] ^= 1;
            assert_eq!(parse_exit_packet(&changed, &nonce), None, "offset {offset}");
        }
    }

    #[test]
    fn exit_packet_rejects_a_stopped_child_as_a_terminal_status() {
        let nonce = [45; 16];
        let stopped = (libc::SIGSTOP << 8) | 0x7f;
        let packet = exit_packet(&nonce, std::process::ExitStatus::from_raw(stopped));
        assert_eq!(parse_exit_packet(&packet, &nonce), None);
    }
}

#[cfg(unix)]
pub(crate) use experiment::run as run_captured_experiment;

#[cfg(unix)]
mod experiment {
    use super::*;
    use crate::capture::{ExperimentSignal, ExperimentStop, NativeExit, Outcome, Request, Stream};
    use std::{
        io::Write,
        os::{
            fd::{AsRawFd, FromRawFd, OwnedFd},
            unix::process::CommandExt,
        },
        process::{Child, Command, Stdio},
        sync::mpsc,
    };

    enum Control {
        Adopt(Child),
        SetupFailed,
        Signal(ExperimentSignal, tokio::sync::oneshot::Sender<Result<()>>),
        Stop { kill: bool },
    }
    struct Keeper {
        commands: Option<mpsc::Sender<Control>>,
        done: tokio::sync::oneshot::Receiver<GuardStatus>,
        status: Option<GuardStatus>,
    }
    impl Keeper {
        fn signal(
            &self,
            signal: ExperimentSignal,
        ) -> Result<tokio::sync::oneshot::Receiver<Result<()>>> {
            let (reply, receive) = tokio::sync::oneshot::channel();
            self.commands
                .as_ref()
                .ok_or_else(|| {
                    GuardStatus::Lost
                        .error()
                        .unwrap_or_else(cleanup_unconfirmed)
                })?
                .send(Control::Signal(signal, reply))
                .map_err(|_| {
                    GuardStatus::Lost
                        .error()
                        .unwrap_or_else(cleanup_unconfirmed)
                })?;
            Ok(receive)
        }
        async fn completed(&mut self) -> GuardStatus {
            if let Some(status) = self.status {
                return status;
            }
            let status = (&mut self.done).await.unwrap_or(GuardStatus::Unconfirmed);
            self.status = Some(status);
            status
        }
        async fn finish(mut self, kill: bool) -> GuardStatus {
            if let Some(commands) = self.commands.take() {
                let _ = commands.send(Control::Stop { kill });
            }
            self.completed().await
        }
    }
    struct Resources {
        anchor: Child,
        observer: Option<Child>,
        anchor_watch: Option<OwnedFd>,
        observer_watch: Option<OwnedFd>,
        health: Pipe,
        receipt: Pipe,
        observed: Option<tokio::sync::oneshot::Sender<NativeObservation>>,
        ready: Option<tokio::sync::oneshot::Sender<()>>,
        native_ready: Option<tokio::sync::oneshot::Sender<()>>,
    }
    struct Pipe {
        fd: OwnedFd,
        nonce: [u8; 16],
        bytes: Vec<u8>,
        eof: bool,
        bad: bool,
    }
    impl Pipe {
        fn poll(&mut self) {
            if self.eof {
                return;
            }
            let mut bytes = [0u8; EXIT_PACKET_LEN + 1];
            let count =
                unsafe { libc::read(self.fd.as_raw_fd(), bytes.as_mut_ptr().cast(), bytes.len()) };
            if count > 0 {
                if self.bytes.len() + count as usize > EXIT_PACKET_LEN * 3 {
                    self.bad = true;
                } else {
                    self.bytes.extend_from_slice(&bytes[..count as usize]);
                }
            } else if count == 0 {
                self.eof = true;
            } else if !matches!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::EAGAIN | libc::EINTR)
            ) {
                self.bad = true;
                self.eof = true;
            }
        }
        fn native(&self) -> Option<NativeObservation> {
            if self.bad {
                return None;
            }
            for packet in self.bytes.as_chunks::<EXIT_PACKET_LEN>().0 {
                let Some(shell_code) = parse_exit_packet(packet, &self.nonce) else {
                    continue;
                };
                use std::os::unix::process::ExitStatusExt;
                let raw = i32::from_be_bytes(packet[24..28].try_into().ok()?);
                let status = std::process::ExitStatus::from_raw(raw);
                return Some(NativeObservation {
                    exit: status.code(),
                    signal: status.signal(),
                    shell_code,
                });
            }
            None
        }
        fn acknowledged(&self) -> bool {
            self.bytes
                .as_chunks::<EXIT_PACKET_LEN>()
                .0
                .iter()
                .any(|packet| is_ack(packet, &self.nonce))
        }
        fn started(&self) -> bool {
            self.bytes
                .get(..EXIT_PACKET_LEN)
                .is_some_and(|packet| is_ready(packet, &self.nonce))
        }
        fn status_valid(&self) -> bool {
            self.valid() && (self.bytes.len() < EXIT_PACKET_LEN || self.started())
        }
        fn valid(&self) -> bool {
            if self.eof && !self.bytes.len().is_multiple_of(EXIT_PACKET_LEN) {
                return false;
            }
            let mut native = 0;
            let mut ack = 0;
            let mut ready = 0;
            for packet in self.bytes.as_chunks::<EXIT_PACKET_LEN>().0 {
                if parse_exit_packet(packet, &self.nonce).is_some() {
                    native += 1;
                } else if is_ack(packet, &self.nonce) {
                    ack += 1;
                } else if is_ready(packet, &self.nonce) {
                    ready += 1;
                } else {
                    return false;
                }
            }
            native <= 1 && ack <= 1 && ready <= 1
        }
    }
    fn acknowledgement(nonce: &[u8; 16]) -> [u8; EXIT_PACKET_LEN] {
        use std::os::unix::process::ExitStatusExt;
        let mut packet = exit_packet(nonce, std::process::ExitStatus::from_raw(0));
        packet[4] = 2;
        packet[24..28].copy_from_slice(&1i32.to_be_bytes());
        packet
    }
    fn is_ack(packet: &[u8], nonce: &[u8; 16]) -> bool {
        packet == acknowledgement(nonce)
    }
    fn ready_packet(nonce: &[u8; 16], pid: u32) -> [u8; EXIT_PACKET_LEN] {
        use std::os::unix::process::ExitStatusExt;
        let mut packet = exit_packet(nonce, std::process::ExitStatus::from_raw(0));
        packet[4] = 3;
        packet[24..28].copy_from_slice(&(pid as i32).to_be_bytes());
        packet
    }
    fn is_ready(packet: &[u8], nonce: &[u8; 16]) -> bool {
        let Some(raw) = packet.get(24..28).and_then(|bytes| bytes.try_into().ok()) else {
            return false;
        };
        let pid = i32::from_be_bytes(raw);
        pid >= 3 && packet == ready_packet(nonce, pid as u32)
    }
    fn nonblocking(fd: &OwnedFd) -> std::io::Result<()> {
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    fn command(
        mode: &str,
        watch: &Watchdog,
        receipt: &Watchdog,
        nonce: &[u8; 16],
    ) -> Result<Command> {
        let read = watch.read.as_raw_fd();
        let write = receipt.write.as_raw_fd();
        let mut command = Command::new(super::guardian()?);
        command
            .arg("internal-guard")
            .arg(mode)
            .arg(read.to_string())
            .arg(write.to_string())
            .arg(uuid::Uuid::from_bytes(*nonce).to_string());
        unsafe {
            command.pre_exec(move || {
                // Sibling groups must share a session so the observer's child
                // can join the independently owned native anchor group.
                if libc::setpgid(0, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                for fd in [read, write] {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        Ok(command)
    }
    fn signal_owned(resources: &Resources, signal: i32, observer: bool) {
        let _ = signal_group(resources.anchor.id() as i32, signal);
        if observer && let Some(child) = &resources.observer {
            let _ = signal_group(child.id() as i32, signal);
        }
    }
    fn own(
        mut resources: Resources,
        controls: mpsc::Receiver<Control>,
        notify: tokio::sync::oneshot::Sender<GuardStatus>,
    ) {
        let mut notify = Some(notify);
        let mut native = None;
        let mut cleanup = None;
        let mut kill_native = false;
        let mut killed = false;
        let mut quiet = 0u8;
        let mut lost = false;
        let mut protocol = false;
        let mut scheduled_kill = false;
        let mut setup_pending = true;
        let mut signal_reply: Option<tokio::sync::oneshot::Sender<Result<()>>> = None;
        loop {
            resources.health.poll();
            resources.receipt.poll();
            protocol |= resources.health.bad
                || resources.receipt.bad
                || !resources.receipt.status_valid()
                || resources.health.bytes.len() > EXIT_PACKET_LEN;
            if !resources.health.bytes.is_empty()
                && resources.health.bytes.len() == EXIT_PACKET_LEN
                && resources
                    .health
                    .native()
                    .is_none_or(|ready| ready.shell_code != 0)
            {
                protocol = true;
            }
            if !protocol
                && let Some(ready) = resources.health.native()
                && ready.shell_code == 0
                && let Some(sender) = resources.ready.take()
            {
                let _ = sender.send(());
            }
            if !protocol
                && resources.receipt.started()
                && let Some(sender) = resources.native_ready.take()
            {
                let _ = sender.send(());
            }
            if !protocol
                && native.is_none()
                && let Some(observed) = resources.receipt.native()
            {
                native = Some(observed);
                if let Some(sender) = resources.observed.take() {
                    let _ = sender.send(observed);
                }
            }
            if !protocol
                && resources.receipt.acknowledged()
                && let Some(reply) = signal_reply.take()
            {
                let _ = reply.send(Ok(()));
            }
            let mut start_cleanup = false;
            match controls.try_recv() {
                Ok(Control::Adopt(child)) => {
                    setup_pending = false;
                    resources.observer = Some(child);
                    if cleanup.is_some() {
                        signal_owned(&resources, libc::SIGCONT, true);
                        signal_owned(&resources, libc::SIGKILL, true);
                    }
                }
                Ok(Control::SetupFailed) => {
                    setup_pending = false;
                    start_cleanup = true;
                    kill_native = true;
                }
                Ok(Control::Signal(signal, reply)) if cleanup.is_none() => match signal {
                    ExperimentSignal::TerminateLeader => {
                        if signal_reply.is_some() {
                            protocol = true;
                            let _ = reply.send(Err(HarnessError::invalid(
                                "Duplicate lifecycle signal attempt.",
                            )));
                        } else if let Some(fd) = &resources.observer_watch {
                            let byte = b"T";
                            if unsafe { libc::write(fd.as_raw_fd(), byte.as_ptr().cast(), 1) } != 1
                            {
                                protocol = true;
                                let _ = reply.send(Err(std::io::Error::last_os_error().into()));
                            } else {
                                signal_reply = Some(reply);
                            }
                        } else {
                            protocol = true;
                            let _ = reply.send(Err(cleanup_unconfirmed()));
                        }
                    }
                    ExperimentSignal::InterruptGroup => {
                        let result = signal_group(resources.anchor.id() as i32, libc::SIGINT)
                            .map_err(HarnessError::from);
                        let _ = reply.send(result);
                    }
                    ExperimentSignal::KillGroup => {
                        scheduled_kill = true;
                        let result = signal_group(resources.anchor.id() as i32, libc::SIGKILL)
                            .map_err(HarnessError::from);
                        let _ = reply.send(result);
                    }
                },
                Ok(Control::Signal(_, reply)) => {
                    let _ = reply.send(Err(GuardStatus::Lost
                        .error()
                        .unwrap_or_else(cleanup_unconfirmed)));
                }
                Ok(Control::Stop { kill }) => {
                    start_cleanup = true;
                    kill_native = kill;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    setup_pending = false;
                    start_cleanup = true;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            if cleanup.is_none() {
                if resources.receipt.eof && resources.observer.is_some()
                    || resources.health.eof && !scheduled_kill
                {
                    lost = true;
                    start_cleanup = true;
                }
                if protocol {
                    start_cleanup = true;
                }
                if start_cleanup {
                    cleanup = Some(Instant::now());
                    resources.anchor_watch.take();
                    resources.observer_watch.take();
                    signal_owned(&resources, libc::SIGCONT, true);
                    signal_owned(
                        &resources,
                        if kill_native {
                            libc::SIGKILL
                        } else {
                            libc::SIGTERM
                        },
                        false,
                    );
                    // Keep the outside observer alive briefly to publish its
                    // actual native wait status after the native group stops.
                }
            }
            if let Some(started) = cleanup {
                if started.elapsed() >= Duration::from_millis(75) {
                    signal_owned(&resources, libc::SIGKILL, true);
                    killed = true;
                }
                let observer_quiet = resources
                    .observer
                    .as_ref()
                    .is_none_or(|child| matches!(group_live(child.id() as i32), Ok(false)));
                if killed
                    && resources.health.eof
                    && resources.receipt.eof
                    && matches!(group_live(resources.anchor.id() as i32), Ok(false))
                    && observer_quiet
                {
                    quiet += 1;
                } else {
                    quiet = 0;
                }
                if quiet >= 2 && !setup_pending {
                    // Both direct-child group leaders still pin their PGIDs.
                    // This is the only reap point; no signal follows it.
                    let anchor_wait = resources.anchor.wait();
                    let observer_wait = resources.observer.as_mut().map(Child::wait).transpose();
                    if anchor_wait.is_err() || observer_wait.is_err() {
                        if let Some(sender) = notify.take() {
                            let _ = sender.send(GuardStatus::Unconfirmed);
                        }
                        std::mem::forget(resources);
                        return;
                    }
                    let result = if protocol {
                        GuardStatus::Protocol
                    } else if lost {
                        GuardStatus::Lost
                    } else {
                        native
                            .map(|value| GuardStatus::Native(value.shell_code))
                            .unwrap_or(GuardStatus::Lost)
                    };
                    if let Some(sender) = notify.take() {
                        let _ = sender.send(result);
                    }
                    return;
                }
                if started.elapsed() >= Duration::from_secs(3)
                    && let Some(sender) = notify.take()
                {
                    let _ = sender.send(GuardStatus::Unconfirmed);
                    // Continue with both actual child pins outside Tokio.
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    type Pipes = (
        Keeper,
        tokio::process::ChildStdin,
        tokio::process::ChildStdout,
        tokio::process::ChildStderr,
        tokio::sync::oneshot::Receiver<NativeObservation>,
    );
    async fn launch(
        request: &Request<'_>,
        interrupted: &mut impl FnMut() -> bool,
    ) -> Result<Pipes> {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        return Err(HarnessError::unavailable(
            "Lifecycle process-group proof is unavailable on this Unix host.",
        ));
        let mut disposition = std::mem::MaybeUninit::<libc::sigaction>::uninit();
        if unsafe { libc::sigaction(libc::SIGCHLD, std::ptr::null(), disposition.as_mut_ptr()) } < 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let disposition = unsafe { disposition.assume_init() };
        if disposition.sa_sigaction == libc::SIG_IGN
            || disposition.sa_flags & libc::SA_NOCLDWAIT != 0
        {
            return Err(HarnessError::unavailable(
                "Lifecycle custody requires retained child wait ownership.",
            ));
        }
        let anchor_watch = Watchdog::new()?;
        let observer_watch = Watchdog::new()?;
        nonblocking(&observer_watch.write)?;
        let observer_writer = observer_watch.write.try_clone()?;
        let anchor_receipt = Watchdog::new()?;
        let observer_receipt = Watchdog::new()?;
        nonblocking(&anchor_receipt.read)?;
        nonblocking(&observer_receipt.read)?;
        let observer_reader = observer_receipt.read.try_clone()?;
        let anchor_nonce = *uuid::Uuid::new_v4().as_bytes();
        let observer_nonce = *uuid::Uuid::new_v4().as_bytes();
        let mut anchor = command(
            "--capture-anchor",
            &anchor_watch,
            &anchor_receipt,
            &anchor_nonce,
        )?;
        anchor
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .current_dir(request.cwd)
            .env_clear()
            .envs(request.environment);
        let (start, receive) = mpsc::sync_channel::<Resources>(1);
        let (send, controls) = mpsc::channel();
        let (notify, done) = tokio::sync::oneshot::channel();
        let (observed, notice) = tokio::sync::oneshot::channel();
        let (ready, readiness) = tokio::sync::oneshot::channel();
        let (native_ready, native_started) = tokio::sync::oneshot::channel();
        // No OS child exists on this fallible owner-thread creation path.
        let _owner = std::thread::Builder::new()
            .name("infra-lifecycle-custody".into())
            .spawn(move || {
                if let Ok(resources) = receive.recv() {
                    own(resources, controls, notify);
                }
            })?;
        let anchor_child = anchor.spawn()?;
        let group = anchor_child.id();
        let resources = Resources {
            anchor: anchor_child,
            observer: None,
            anchor_watch: Some(anchor_watch.retain_parent()),
            observer_watch: Some(observer_writer),
            health: Pipe {
                fd: anchor_receipt.retain_reader(),
                nonce: anchor_nonce,
                bytes: Vec::new(),
                eof: false,
                bad: false,
            },
            receipt: Pipe {
                fd: observer_reader,
                nonce: observer_nonce,
                bytes: Vec::new(),
                eof: false,
                bad: false,
            },
            observed: Some(observed),
            ready: Some(ready),
            native_ready: Some(native_ready),
        };
        // Adopt the first child before the first await or callback. Pending
        // setup prevents its reap even if it dies before observer admission.
        if let Err(error) = start.send(resources) {
            signal_owned(&error.0, libc::SIGKILL, true);
            std::mem::forget(error.0);
            return Err(cleanup_unconfirmed());
        }
        drop(start);
        let mut keeper = Keeper {
            commands: Some(send),
            done,
            status: None,
        };
        let ready=async {
            let deadline=Instant::now().checked_add(Duration::from_secs(1)).unwrap_or(request.deadline).min(request.deadline);
            tokio::pin!(readiness);
            loop {
                if interrupted(){return Err(crate::capture::cancelled());}
                if Instant::now()>=deadline{return Err(crate::capture::timeout());}
                tokio::select! {
                    result=&mut readiness=>return result.map_err(|_|HarnessError::unavailable("Lifecycle anchor did not initialize.")),
                    status=keeper.completed()=>return Err(status.error().unwrap_or_else(cleanup_unconfirmed)),
                    _=tokio::time::sleep(Duration::from_millis(5))=>{},
                }
            }
        }.await;
        let prepared = (|| -> Result<Child> {
            ready?;
            let mut observer = command(
                "--capture-observer",
                &observer_watch,
                &observer_receipt,
                &observer_nonce,
            )?;
            observer
                .arg(group.to_string())
                .arg("--")
                .args(request.argv)
                .current_dir(request.cwd)
                .env_clear()
                .envs(request.environment)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            Ok(observer.spawn()?)
        })();
        let (mut observer, setup_error) = match prepared {
            Ok(child) => (Some(child), None),
            Err(error) => (None, Some(error)),
        };
        let stdin = observer.as_mut().and_then(|child| child.stdin.take());
        let stdout = observer.as_mut().and_then(|child| child.stdout.take());
        let stderr = observer.as_mut().and_then(|child| child.stderr.take());
        let control = observer.map(Control::Adopt).unwrap_or(Control::SetupFailed);
        let adopted = keeper
            .commands
            .as_ref()
            .expect("live lifecycle setup sender")
            .send(control);
        // These parent duplicates must close before owner cleanup can prove EOF.
        drop(observer_watch);
        drop(observer_receipt);
        if let Err(mpsc::SendError(control)) = adopted {
            if let Control::Adopt(child) = control {
                let _ = signal_group(child.id() as i32, libc::SIGKILL);
                std::mem::forget(child);
            }
            return Err(cleanup_unconfirmed());
        }
        let pipes = (|| -> Result<_> {
            if let Some(error) = setup_error {
                return Err(error);
            }
            Ok((
                tokio::process::ChildStdin::from_std(stdin.ok_or_else(|| {
                    HarnessError::unavailable("Lifecycle stdin is unavailable.")
                })?)?,
                tokio::process::ChildStdout::from_std(stdout.ok_or_else(|| {
                    HarnessError::unavailable("Lifecycle stdout is unavailable.")
                })?)?,
                tokio::process::ChildStderr::from_std(stderr.ok_or_else(|| {
                    HarnessError::unavailable("Lifecycle stderr is unavailable.")
                })?)?,
            ))
        })();
        match pipes {
            Ok((stdin, stdout, stderr)) => {
                // Helper exec alone is not native admission. Wait until its
                // native spawn has joined the pinned group and crossed exec.
                let admission=async {
                    tokio::pin!(native_started);
                    loop {
                        if interrupted(){return Err(crate::capture::cancelled());}
                        if Instant::now()>=request.deadline{return Err(crate::capture::timeout());}
                        tokio::select! {
                            result=&mut native_started=>return result.map_err(|_|GuardStatus::Lost.error().unwrap_or_else(cleanup_unconfirmed)),
                            status=keeper.completed()=>return Err(status.error().unwrap_or_else(cleanup_unconfirmed)),
                            _=tokio::time::sleep(Duration::from_millis(5))=>{},
                        }
                    }
                }.await;
                if let Err(error) = admission {
                    drop(stdin);
                    drop(stdout);
                    drop(stderr);
                    if keeper.finish(true).await == GuardStatus::Unconfirmed {
                        return Err(cleanup_unconfirmed());
                    }
                    return Err(error);
                }
                Ok((keeper, stdin, stdout, stderr, notice))
            }
            Err(error) => {
                if keeper.finish(true).await == GuardStatus::Unconfirmed {
                    return Err(cleanup_unconfirmed());
                }
                Err(error)
            }
        }
    }
    fn descriptors(arguments: &[String], count: usize) -> Result<(i32, std::fs::File, [u8; 16])> {
        if arguments.len() < count {
            return Err(HarnessError::invalid(
                "Incomplete lifecycle guardian invocation.",
            ));
        }
        let watch = arguments[0]
            .parse::<i32>()
            .ok()
            .filter(|fd| *fd >= 3)
            .ok_or_else(|| HarnessError::invalid("Invalid lifecycle watchdog."))?;
        let receipt = arguments[1]
            .parse::<i32>()
            .map_err(|_| HarnessError::invalid("Invalid lifecycle receipt."))?;
        let nonce = *uuid::Uuid::parse_str(&arguments[2])
            .map_err(|_| HarnessError::invalid("Invalid lifecycle nonce."))?
            .as_bytes();
        guard_descriptor_custody(watch, receipt, None)?;
        for signal in [libc::SIGTERM, libc::SIGHUP] {
            if unsafe { libc::signal(signal, guard_signal as *const () as libc::sighandler_t) }
                == libc::SIG_ERR
            {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        if unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) } == libc::SIG_ERR
            || unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) } == libc::SIG_ERR
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut empty = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
        if unsafe { libc::sigemptyset(empty.as_mut_ptr()) } < 0
            || unsafe { libc::sigprocmask(libc::SIG_SETMASK, empty.as_ptr(), std::ptr::null_mut()) }
                < 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok((watch, unsafe { std::fs::File::from_raw_fd(receipt) }, nonce))
    }
    pub(super) fn anchor_entry(arguments: &[String]) -> Result<i32> {
        if arguments.len() != 3 {
            return Err(HarnessError::invalid(
                "Invalid lifecycle anchor invocation.",
            ));
        }
        let (watch, mut receipt, nonce) = descriptors(arguments, 3)?;
        let group = unsafe { libc::getpid() };
        if unsafe { libc::getpgrp() } != group {
            return Err(HarnessError::invalid(
                "Lifecycle anchor does not own its group.",
            ));
        }
        use std::os::unix::process::ExitStatusExt;
        if receipt
            .write_all(&exit_packet(&nonce, std::process::ExitStatus::from_raw(0)))
            .is_err()
            || receipt.flush().is_err()
        {
            guard_end_group(group);
        }
        loop {
            if GUARD_TERMINATING.load(Ordering::Acquire) {
                guard_end_group(group);
            }
            let mut poll = libc::pollfd {
                fd: watch,
                events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut poll, 1, 20) };
            if ready < 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
            {
                guard_end_group(group);
            }
            if poll.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
                guard_end_group(group);
            }
            if poll.revents & libc::POLLIN != 0 {
                let mut bytes = [0u8; 1];
                if unsafe { libc::read(watch, bytes.as_mut_ptr().cast(), 1) } <= 0 {
                    guard_end_group(group);
                }
            }
        }
    }
    fn observer_publish(
        child: &mut Child,
        receipt: &mut std::fs::File,
        nonce: &[u8; 16],
        observed: &mut bool,
    ) -> std::io::Result<()> {
        if !*observed && let Some(status) = child.try_wait()? {
            // Reaping and publication are separate: parent loss may close the
            // receipt reader, but must not keep this trusted observer alive.
            *observed = true;
            receipt.write_all(&exit_packet(nonce, status))?;
            receipt.flush()?;
        }
        Ok(())
    }
    pub(super) fn observer_entry(arguments: &[String]) -> Result<i32> {
        if arguments.len() < 6 || arguments[4] != "--" {
            return Err(HarnessError::invalid(
                "Invalid lifecycle observer invocation.",
            ));
        }
        let (watch, mut receipt, nonce) = descriptors(arguments, 6)?;
        let group = arguments[3]
            .parse::<i32>()
            .ok()
            .filter(|group| *group > 2)
            .ok_or_else(|| HarnessError::invalid("Invalid lifecycle native group."))?;
        if unsafe { libc::getpgrp() } != unsafe { libc::getpid() }
            || group == unsafe { libc::getpgrp() }
            || unsafe { libc::getpgid(group) } != group
        {
            return Err(HarnessError::invalid(
                "Lifecycle native group is not anchored.",
            ));
        }
        let mut command = Command::new(&arguments[5]);
        command.args(&arguments[6..]);
        unsafe {
            command.pre_exec(move || {
                if libc::setpgid(0, group) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGPIPE] {
                    if libc::signal(signal, libc::SIG_DFL) == libc::SIG_ERR {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                let mut empty = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
                if libc::sigemptyset(empty.as_mut_ptr()) < 0
                    || libc::sigprocmask(libc::SIG_SETMASK, empty.as_ptr(), std::ptr::null_mut())
                        < 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn()?;
        for fd in [0, 1, 2] {
            unsafe {
                libc::close(fd);
            }
        }
        let mut sent = false;
        // Emit admission only after native pre_exec group join and exec have
        // succeeded. Publication failure still retains this actual Child.
        let mut closing = if receipt
            .write_all(&ready_packet(&nonce, child.id()))
            .is_err()
            || receipt.flush().is_err()
        {
            Some(Instant::now())
        } else {
            None
        };
        loop {
            if observer_publish(&mut child, &mut receipt, &nonce, &mut sent).is_err() {
                closing.get_or_insert(Instant::now());
            }
            if GUARD_TERMINATING.load(Ordering::Acquire) {
                closing.get_or_insert(Instant::now());
            }
            if closing.is_some() && sent {
                return Ok(0);
            }
            // If parent loss outlives trusted anchor cleanup, terminate only
            // this still-unreaped native child; never a saved foreign PGID.
            if closing.is_some_and(|at| at.elapsed() >= Duration::from_secs(3)) && !sent {
                let _ = child.kill();
            }
            let mut poll = libc::pollfd {
                fd: watch,
                events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut poll, 1, 10) };
            if ready < 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
            {
                closing.get_or_insert(Instant::now());
            }
            if poll.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
                closing.get_or_insert(Instant::now());
            }
            if poll.revents & libc::POLLIN != 0 {
                let mut command = [0u8; 1];
                let count = unsafe { libc::read(watch, command.as_mut_ptr().cast(), 1) };
                if count <= 0 {
                    closing.get_or_insert(Instant::now());
                } else if command[0] == b'T' {
                    // Poll first: a reaped native PID is never signalled.
                    observer_publish(&mut child, &mut receipt, &nonce, &mut sent)?;
                    if !sent && unsafe { libc::kill(child.id() as i32, libc::SIGTERM) } < 0 {
                        let error = std::io::Error::last_os_error();
                        if error.raw_os_error() != Some(libc::ESRCH) {
                            return Err(error.into());
                        }
                    }
                    receipt.write_all(&acknowledgement(&nonce))?;
                    receipt.flush()?;
                } else {
                    return Err(HarnessError::invalid("Unknown lifecycle control."));
                }
            }
        }
    }
    pub(crate) async fn run(
        request: Request<'_>,
        stop: ExperimentStop,
        mut interrupted: impl FnMut() -> bool + Send,
        mut observed_exit: impl FnMut(NativeExit) + Send,
        mut event: impl FnMut(Stream, &[u8]) -> Result<()> + Send,
    ) -> Result<Outcome> {
        validate_command(request.argv, &[])?;
        if interrupted() {
            return Err(crate::capture::cancelled());
        }
        let (mut keeper, mut input, mut stdout, mut stderr, mut notice) =
            launch(&request, &mut interrupted).await?;
        let started = Instant::now();
        let Some(schedule) = started.checked_add(stop.after) else {
            if keeper.finish(true).await == GuardStatus::Unconfirmed {
                return Err(cleanup_unconfirmed());
            }
            return Err(HarnessError::invalid("Invalid lifecycle signal deadline."));
        };
        let prompt = request.input.to_vec();
        let admitted = !interrupted() && Instant::now() < request.deadline;
        let mut writer = tokio::spawn(async move {
            if admitted
                && let Err(error) = input.write_all(&prompt).await
                && error.kind() != std::io::ErrorKind::BrokenPipe
            {
                return Err(HarnessError::from(error));
            }
            if let Err(error) = input.shutdown().await
                && error.kind() != std::io::ErrorKind::BrokenPipe
            {
                return Err(HarnessError::from(error));
            }
            Ok::<(), HarnessError>(())
        });
        let mut writer_done = false;
        let mut stdout_open = true;
        let mut stderr_open = true;
        let mut native = None;
        let mut observation_open = true;
        let mut total = 0usize;
        let mut grace = None;
        let mut force_kill = false;
        let mut signal_sent = false;
        let mut pending_ack = None;
        let execution:Result<()>=async {
            loop {
                if interrupted(){return Err(crate::capture::cancelled());}
                let now=Instant::now();
                if now>=request.deadline{return Err(crate::capture::timeout());}
                if !stdout_open && !stderr_open && native.is_some() && writer_done{return Ok(());}
                if !signal_sent && now>=schedule {
                    pending_ack=Some(keeper.signal(stop.signal)?);signal_sent=true;
                }
                if grace.is_some_and(|deadline|now>=deadline){force_kill=true;return Ok(());}
                let mut out=[0u8;65536];let mut err=[0u8;65536];
                let read=tokio::select!{
                    acknowledged=async {pending_ack.as_mut().expect("pending lifecycle acknowledgement").await},if pending_ack.is_some()=>{
                        acknowledged.map_err(|_|GuardStatus::Lost.error().unwrap_or_else(cleanup_unconfirmed))??;pending_ack=None;
                        grace=Some(Instant::now().checked_add(stop.grace).ok_or_else(||HarnessError::invalid("Invalid lifecycle grace deadline."))?);
                        continue;
                    },
                    status=keeper.completed(),if keeper.status.is_none()=>{
                        return Err(status.error().unwrap_or_else(cleanup_unconfirmed));
                    },
                    result=&mut writer,if !writer_done=>{
                        writer_done=true;
                        result.map_err(|_|HarnessError::unavailable("Lifecycle input worker stopped."))??;
                        continue;
                    },
                    observed=&mut notice,if observation_open=>{
                        observation_open=false;native=observed.ok();
                        if let Some(value)=native{observed_exit(NativeExit{code:value.exit,signal:value.signal,shell_code:value.shell_code});}
                        continue;
                    },
                    result=stdout.read(&mut out),if stdout_open=>(Stream::Stdout,result.map(|n|out[..n].to_vec())),
                    result=stderr.read(&mut err),if stderr_open=>(Stream::Stderr,result.map(|n|err[..n].to_vec())),
                    _=tokio::time::sleep(Duration::from_millis(5))=>continue,
                };
                let(stream,bytes)=(read.0,read.1?);
                if bytes.is_empty(){match stream{Stream::Stdout=>stdout_open=false,Stream::Stderr=>stderr_open=false};continue;}
                total=total.checked_add(bytes.len()).filter(|n|*n<=request.output_limit).ok_or_else(crate::capture::overflow)?;
                event(stream,&bytes)?;
            }
        }.await;
        let status = keeper.finish(force_kill).await;
        if observation_open && let Ok(value) = notice.try_recv() {
            native = Some(value);
            observed_exit(NativeExit {
                code: value.exit,
                signal: value.signal,
                shell_code: value.shell_code,
            });
        }
        if status == GuardStatus::Unconfirmed {
            writer.abort();
            let _ = writer.await;
            return Err(cleanup_unconfirmed());
        }
        let fed = if writer_done {
            Ok(())
        } else {
            match tokio::time::timeout(Duration::from_secs(1), &mut writer).await {
                Ok(Ok(value)) => value,
                Ok(Err(_)) => Err(HarnessError::unavailable("Lifecycle input worker stopped.")),
                Err(_) => {
                    writer.abort();
                    let _ = writer.await;
                    Err(HarnessError::unavailable(
                        "Lifecycle stdin did not close after cleanup.",
                    ))
                }
            }
        };
        execution?;
        fed?;
        if force_kill {
            // Grace expiry still drains the now-closed pipes, as communicate
            // does after the final KILL. No process remains able to hold them.
            let deadline = Instant::now() + Duration::from_secs(1);
            while stdout_open || stderr_open {
                let mut out = [0u8; 65536];
                let mut err = [0u8; 65536];
                let read = tokio::select! {
                    result=stdout.read(&mut out),if stdout_open=>(Stream::Stdout,result.map(|n|out[..n].to_vec())),
                    result=stderr.read(&mut err),if stderr_open=>(Stream::Stderr,result.map(|n|err[..n].to_vec())),
                    _=tokio::time::sleep_until(tokio::time::Instant::from_std(deadline))=>return Err(HarnessError::unavailable("Lifecycle streams did not close after proven cleanup.")),
                };
                let (stream, bytes) = (read.0, read.1?);
                if bytes.is_empty() {
                    match stream {
                        Stream::Stdout => stdout_open = false,
                        Stream::Stderr => stderr_open = false,
                    };
                    continue;
                }
                total = total
                    .checked_add(bytes.len())
                    .filter(|n| *n <= request.output_limit)
                    .ok_or_else(crate::capture::overflow)?;
                event(stream, &bytes)?;
            }
        }
        let GuardStatus::Native(code) = status else {
            return Err(status.error().unwrap_or_else(cleanup_unconfirmed));
        };
        let native = native.ok_or_else(|| {
            GuardStatus::Lost
                .error()
                .unwrap_or_else(cleanup_unconfirmed)
        })?;
        if native.shell_code != code {
            return Err(GuardStatus::Protocol
                .error()
                .unwrap_or_else(cleanup_unconfirmed));
        }
        Ok(Outcome {
            exit: native.exit,
            signal: native.signal,
            shell_code: code,
            seconds: started.elapsed().as_secs_f64(),
        })
    }
    #[cfg(test)]
    mod frame_tests {
        use super::*;
        use std::os::unix::process::ExitStatusExt;
        fn pipe(bytes: Vec<u8>, eof: bool) -> Pipe {
            Pipe {
                fd: Watchdog::new().unwrap().retain_reader(),
                nonce: [67; 16],
                bytes,
                eof,
                bad: false,
            }
        }
        #[test]
        fn ready_exit_ack_frames_are_nonce_bound_and_allow_both_exit_orders() {
            let nonce = [67; 16];
            let ready = ready_packet(&nonce, 314);
            let exit = exit_packet(&nonce, std::process::ExitStatus::from_raw(125 << 8));
            let ack = acknowledgement(&nonce);
            for frames in [[ready, exit, ack], [ready, ack, exit]] {
                let p = pipe(frames.concat(), true);
                assert!(p.status_valid());
                assert!(p.started());
                assert!(p.acknowledged());
                assert_eq!(p.native().unwrap().exit, Some(125));
            }
            assert!(!pipe([exit, ready].concat(), true).status_valid());
        }
        #[test]
        fn status_frames_reject_unknown_duplicate_nonce_and_truncated_eof() {
            let nonce = [67; 16];
            let ready = ready_packet(&nonce, 314);
            let exit = exit_packet(&nonce, std::process::ExitStatus::from_raw(0));
            let ack = acknowledgement(&nonce);
            for frames in [[ready, ready], [ready, exit], [ready, ack]] {
                let mut duplicate = frames.concat();
                duplicate.extend_from_slice(&frames[1]);
                assert!(!pipe(duplicate, true).status_valid());
            }
            for length in 1..EXIT_PACKET_LEN {
                let mut partial = ready.to_vec();
                partial.extend_from_slice(&exit[..length]);
                assert!(!pipe(partial, true).status_valid());
            }
            for offset in [0, 4, 5, 8, 28, 31] {
                let mut corrupt = exit;
                corrupt[offset] ^= 1;
                assert!(!pipe([ready, corrupt].concat(), true).status_valid());
            }
            assert!(
                !pipe(
                    [
                        ready,
                        exit_packet(&[68; 16], std::process::ExitStatus::from_raw(0))
                    ]
                    .concat(),
                    true
                )
                .status_valid()
            );
        }
    }
}

#[cfg(all(test, unix))]
mod line_framing_tests {
    use super::{HarnessError, Result, emit_lines};

    // Frozen framing loop before the cursor optimization. The reference owns
    // every packet and removes it before invoking the callback.
    fn reference_lines(
        line: &mut Vec<u8>,
        limit: usize,
        event: &mut impl FnMut(&[u8]) -> Result<()>,
    ) -> Result<()> {
        while let Some(end) = line.iter().position(|byte| *byte == b'\n') {
            if end > limit {
                return Err(HarnessError {
                    status: 503,
                    code: "output-line-limit",
                    message: "Native output line limit reached.".into(),
                });
            }
            let packet = line.drain(..=end).collect::<Vec<_>>();
            event(&packet)?;
        }
        if line.len() > limit {
            return Err(HarnessError {
                status: 503,
                code: "output-line-limit",
                message: "Native output line limit reached.".into(),
            });
        }
        Ok(())
    }

    #[derive(Debug, Eq, PartialEq)]
    struct Image {
        packets: Vec<Vec<u8>>,
        residual: Vec<u8>,
        error: Option<(u16, &'static str, String)>,
    }

    fn error_image(error: HarnessError) -> (u16, &'static str, String) {
        (error.status, error.code, error.message)
    }

    fn feed(chunks: &[&[u8]], limit: usize, fail_at: Option<usize>, reference: bool) -> Image {
        let mut line = Vec::new();
        let mut packets = Vec::new();
        let mut error = None;
        let mut event = |packet: &[u8]| {
            let index = packets.len();
            packets.push(packet.to_vec());
            if fail_at == Some(index) {
                return Err(HarnessError {
                    status: 409,
                    code: "framing-callback-fixture",
                    message: "Frozen callback failure.".into(),
                });
            }
            Ok(())
        };
        for chunk in chunks.iter().filter(|chunk| !chunk.is_empty()) {
            line.extend_from_slice(chunk);
            let result = if reference {
                reference_lines(&mut line, limit, &mut event)
            } else {
                emit_lines(&mut line, limit, &mut event)
            };
            if let Err(failure) = result {
                error = Some(error_image(failure));
                break;
            }
        }
        // run_engine delivers the remaining unterminated packet at EOF,
        // without removing it, only after every read passed its line limit.
        if error.is_none()
            && !line.is_empty()
            && let Err(failure) = event(&line)
        {
            error = Some(error_image(failure));
        }
        Image {
            packets,
            residual: line,
            error,
        }
    }

    #[test]
    fn cursor_framing_matches_frozen_packets_errors_and_residuals_at_all_splits() {
        for payload in [b"\nA\r\n\0\xff\nTAIL".as_slice(), b"\xc3\xa5\nZ".as_slice()] {
            for first in 0..=payload.len() {
                for second in first..=payload.len() {
                    let chunks = [
                        &payload[..first],
                        &payload[first..second],
                        &payload[second..],
                    ];
                    for limit in [0, 1, 2, 3, 4, usize::MAX] {
                        for fail_at in [None, Some(0), Some(1), Some(2), Some(3)] {
                            assert_eq!(
                                feed(&chunks, limit, fail_at, false),
                                feed(&chunks, limit, fail_at, true),
                                "payload={payload:?}, cuts={first}/{second}, limit={limit}, fail={fail_at:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn packet_boundaries_include_lf_cr_binary_and_unterminated_eof() {
        let image = feed(&[b"\nAB\r\n\0\xff\nTAIL"], 4, None, false);
        assert_eq!(
            image,
            Image {
                packets: vec![
                    b"\n".to_vec(),
                    b"AB\r\n".to_vec(),
                    b"\0\xff\n".to_vec(),
                    b"TAIL".to_vec()
                ],
                residual: b"TAIL".to_vec(),
                error: None,
            }
        );
        assert_eq!(
            feed(&[b"AB\n"], 2, None, false).packets,
            vec![b"AB\n".to_vec()]
        );
        assert_eq!(feed(&[b"AB"], 2, None, false).packets, vec![b"AB".to_vec()]);
        assert!(feed(&[], 0, None, false).packets.is_empty());
    }

    #[test]
    fn failures_keep_exact_error_and_previously_consumed_prefix_state() {
        let expected_error = Some((
            503,
            "output-line-limit",
            "Native output line limit reached.".into(),
        ));
        for payload in [b"ok\nLONG\nlater\nTAIL".as_slice(), b"ok\nLONG".as_slice()] {
            let image = feed(&[payload], 3, None, false);
            assert_eq!(image.packets, vec![b"ok\n".to_vec()]);
            assert_eq!(image.residual, payload[3..]);
            assert_eq!(image.error, expected_error);
        }
        assert_eq!(
            feed(&[b"one\ntwo\nthree\n"], usize::MAX, Some(1), false),
            Image {
                packets: vec![b"one\n".to_vec(), b"two\n".to_vec()],
                residual: b"three\n".to_vec(),
                error: Some((
                    409,
                    "framing-callback-fixture",
                    "Frozen callback failure.".into()
                )),
            }
        );
    }
}

#[cfg(test)]
mod guardian_tests {
    #[test]
    fn a_relative_guardian_is_refused_and_this_executable_still_guards() {
        assert!(super::use_guardian("infra-engine".into()).is_err());
        assert_eq!(super::guardian().unwrap(), std::env::current_exe().unwrap());
    }
}
