//! Bounded native commands share owned process custody with the host runtime.
use infra_core::{ErrorKind, Result, RuntimeError};
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    io::{Read, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
#[derive(Debug)]
pub struct ProcessOutput {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}
thread_local! {static CANCEL:std::cell::RefCell<Option<Arc<AtomicBool>>>=const{std::cell::RefCell::new(None)};}
pub fn with_cancel<T>(cancel: Arc<AtomicBool>, work: impl FnOnce() -> T) -> T {
    struct Restore(Option<Arc<AtomicBool>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CANCEL.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let saved = CANCEL.with(|slot| slot.replace(Some(cancel)));
    let _restore = Restore(saved);
    work()
}
fn coded(code: &str, message: &str) -> RuntimeError {
    RuntimeError::coded(ErrorKind::Unavailable, code, message)
}
#[cfg(windows)]
fn host_error(error: infra_host::HarnessError) -> RuntimeError {
    RuntimeError::coded(ErrorKind::Unavailable, error.code, error.message)
}
pub(crate) struct OwnedChild {
    pub(crate) stdin: Option<Box<dyn Write + Send>>,
    pub(crate) stdout: Option<Box<dyn Read + Send>>,
    pub(crate) stderr: Option<Box<dyn Read + Send>>,
    #[cfg(unix)]
    child: std::process::Child,
    #[cfg(unix)]
    group: i32,
    /// The stdout and stderr descriptors behind the boxed pipes, or -1.
    #[cfg(unix)]
    fds: [i32; 2],
    #[cfg(windows)]
    child: infra_host::process::GuardedChild,
    finished: bool,
}
#[cfg(unix)]
fn nonblocking(pipe: &impl std::os::fd::AsRawFd) -> Result<()> {
    let fd = pipe.as_raw_fd();
    let current = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if current < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, current | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
pub(crate) fn spawn_child(
    program: &OsStr,
    args: &[OsString],
    cwd: &Path,
    input: bool,
    capture_stderr: bool,
    environment: &BTreeMap<OsString, Option<OsString>>,
) -> Result<OwnedChild> {
    #[cfg(unix)]
    {
        use std::{
            os::unix::process::CommandExt,
            process::{Command, Stdio},
        };
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(cwd)
            .stdin(if input { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(if capture_stderr {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        // Unlike a pre-exec hook, a process group lets the standard library
        // spawn without forking this process.
        command.process_group(0);
        for (key, value) in environment {
            match value {
                Some(value) => {
                    command.env(key, value);
                }
                None => {
                    command.env_remove(key);
                }
            }
        }
        let child = command.spawn().map_err(|error| {
            let mut failure = RuntimeError::coded(
                ErrorKind::Unavailable,
                "native-spawn",
                format!("Native command could not start: {error}"),
            );
            failure.os_code = error.raw_os_error();
            failure
        })?;
        let group = child.id() as i32;
        let mut owned = OwnedChild {
            child,
            group,
            stdin: None,
            stdout: None,
            stderr: None,
            fds: [-1, -1],
            finished: false,
        };
        if let Some(pipe) = owned.child.stdin.take() {
            nonblocking(&pipe)?;
            owned.stdin = Some(Box::new(pipe));
        }
        if let Some(pipe) = owned.child.stdout.take() {
            nonblocking(&pipe)?;
            owned.fds[0] = std::os::fd::AsRawFd::as_raw_fd(&pipe);
            owned.stdout = Some(Box::new(pipe));
        }
        if let Some(pipe) = owned.child.stderr.take() {
            nonblocking(&pipe)?;
            owned.fds[1] = std::os::fd::AsRawFd::as_raw_fd(&pipe);
            owned.stderr = Some(Box::new(pipe));
        }
        Ok(owned)
    }
    #[cfg(windows)]
    {
        let complete = complete_environment(environment)?;
        let mut argv = vec![
            program
                .to_str()
                .ok_or_else(|| RuntimeError::invalid("Native executable is not Unicode"))?
                .to_string(),
        ];
        for argument in args {
            argv.push(
                argument
                    .to_str()
                    .ok_or_else(|| RuntimeError::invalid("Native argument is not Unicode"))?
                    .into(),
            );
        }
        let mut child =
            infra_host::process::GuardedChild::spawn(&argv, cwd, &complete, None, capture_stderr)
                .map_err(host_error)?;
        let stdin = if input {
            child
                .stdin
                .take()
                .map(|pipe| Box::new(pipe) as Box<dyn Write + Send>)
        } else {
            child.stdin.take();
            None
        };
        let stdout = child
            .stdout
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>);
        let stderr = child
            .stderr
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>);
        Ok(OwnedChild {
            child,
            stdin,
            stdout,
            stderr,
            finished: false,
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (program, args, cwd, input, capture_stderr, environment);
        Err(RuntimeError::unavailable(
            "Owned native process execution is unsupported on this platform",
        ))
    }
}
impl OwnedChild {
    pub(crate) fn try_wait(&mut self) -> Result<Option<i32>> {
        #[cfg(unix)]
        {
            Ok(self
                .child
                .try_wait()?
                .map(|status| status.code().unwrap_or(-1)))
        }
        #[cfg(windows)]
        {
            self.child.try_wait().map_err(host_error)
        }
        #[cfg(not(any(unix, windows)))]
        {
            Err(RuntimeError::unavailable(
                "Owned native process execution is unsupported",
            ))
        }
    }
    /// Terminate descendants even when the command leader has already exited.
    pub(crate) fn terminate(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.stdin.take();
        #[cfg(unix)]
        {
            let result = unsafe { libc::kill(-self.group, libc::SIGKILL) };
            if result < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
                return Err(coded(
                    "cleanup-unconfirmed",
                    "Native process group cleanup could not be confirmed",
                ));
            }
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                match self.child.try_wait() {
                    Ok(Some(_)) => {
                        self.finished = true;
                        return Ok(());
                    }
                    Ok(None) if Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    _ => {
                        return Err(coded(
                            "cleanup-unconfirmed",
                            "Native process leader cleanup could not be confirmed",
                        ));
                    }
                }
            }
        }
        #[cfg(windows)]
        {
            self.child
                .kill_and_wait(Duration::from_secs(3))
                .map_err(host_error)?;
            self.finished = true;
            Ok(())
        }
        #[cfg(not(any(unix, windows)))]
        {
            Err(RuntimeError::unavailable(
                "Owned native process cleanup is unsupported",
            ))
        }
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}
/// Read a pipe to its end, then wake `waiter`: the end of a child's output
/// usually arrives with its exit, which the waiter would otherwise only notice
/// on its next look.
fn read_pipe(
    pipe: Box<dyn Read + Send>,
    fd: i32,
    waiter: thread::Thread,
    limit: usize,
    stop: Arc<AtomicBool>,
    overflow: Arc<AtomicBool>,
) -> std::io::Result<Vec<u8>> {
    let result = read_pipe_until_end(pipe, fd, limit, stop, overflow);
    waiter.unpark();
    result
}
/// Wait until `fd` is readable, at its end or `pause` has passed.
fn pipe_wait(fd: i32, pause: Duration) {
    #[cfg(unix)]
    if fd >= 0 {
        let mut entry = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let milliseconds = pause.as_millis().clamp(1, 1000) as libc::c_int;
        // Interrupted or failed waits only end this pause early.
        unsafe { libc::poll(&mut entry, 1, milliseconds) };
        return;
    }
    let _ = fd;
    thread::sleep(pause);
}
fn read_pipe_until_end(
    mut pipe: Box<dyn Read + Send>,
    fd: i32,
    limit: usize,
    stop: Arc<AtomicBool>,
    overflow: Arc<AtomicBool>,
) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut stopping = None;
    // Output usually ends just before the exit: look again soon after data.
    let mut pause = Duration::from_micros(100);
    loop {
        if stop.load(Ordering::Acquire) {
            let since = stopping.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_millis(50) {
                return Ok(bytes);
            }
        }
        match pipe.read(&mut chunk) {
            Ok(0) => return Ok(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => return Ok(bytes),
            Ok(count) => {
                pause = Duration::from_micros(100);
                let remaining = (limit + 1).saturating_sub(bytes.len());
                bytes.extend_from_slice(&chunk[..count.min(remaining)]);
                if bytes.len() > limit {
                    overflow.store(true, Ordering::Release);
                    return Ok(bytes);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if stop.load(Ordering::Acquire) {
                    return Ok(bytes);
                }
                pipe_wait(fd, pause);
                pause = (pause * 3 / 2).min(Duration::from_millis(2));
            }
            Err(error) => return Err(error),
        }
    }
}
pub(crate) fn write_pipe(
    mut pipe: Box<dyn Write + Send>,
    bytes: Vec<u8>,
    stop: Arc<AtomicBool>,
) -> std::io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        if stop.load(Ordering::Acquire) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "Native input stopped",
            ));
        }
        match pipe.write(&bytes[offset..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "Native input closed",
                ));
            }
            Ok(count) => offset += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2))
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
pub fn run(
    program: &OsStr,
    args: &[OsString],
    cwd: &Path,
    input: Option<Vec<u8>>,
    timeout: Duration,
    limit: usize,
) -> Result<ProcessOutput> {
    run_with_env(program, args, cwd, input, timeout, limit, &BTreeMap::new())
}
pub fn run_with_env(
    program: &OsStr,
    args: &[OsString],
    cwd: &Path,
    input: Option<Vec<u8>>,
    timeout: Duration,
    limit: usize,
    environment: &BTreeMap<OsString, Option<OsString>>,
) -> Result<ProcessOutput> {
    if timeout.is_zero()
        || limit == 0
        || limit > 512 * 1024 * 1024
        || input
            .as_ref()
            .is_some_and(|bytes| bytes.len() > 512 * 1024 * 1024)
    {
        return Err(RuntimeError::invalid("Invalid native process budget"));
    }
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| RuntimeError::invalid("Native process time limit cannot be represented"))?;
    let mut child = spawn_child(program, args, cwd, input.is_some(), true, environment)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| RuntimeError::unavailable("Native stdout unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| RuntimeError::unavailable("Native stderr unavailable"))?;
    let stop = Arc::new(AtomicBool::new(false));
    let overflow = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    let [out_fd, err_fd] = child.fds;
    #[cfg(not(unix))]
    let [out_fd, err_fd] = [-1, -1];
    let waiter = thread::current();
    let out = {
        let stop = stop.clone();
        let overflow = overflow.clone();
        let waiter = waiter.clone();
        thread::spawn(move || read_pipe(stdout, out_fd, waiter, limit, stop, overflow))
    };
    let err = {
        let stop = stop.clone();
        let overflow = overflow.clone();
        thread::spawn(move || read_pipe(stderr, err_fd, waiter, limit, stop, overflow))
    };
    let writer = if let Some(bytes) = input {
        let pipe = child
            .stdin
            .take()
            .ok_or_else(|| RuntimeError::unavailable("Native stdin unavailable"))?;
        let stop = stop.clone();
        Some(thread::spawn(move || write_pipe(pipe, bytes, stop)))
    } else {
        None
    };
    let cancellation = CANCEL.with(|slot| slot.borrow().clone());
    let started = Instant::now();
    let completed = loop {
        if cancellation
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Acquire))
        {
            break Err(coded("native-cancelled", "Native command was cancelled"));
        }
        if overflow.load(Ordering::Acquire) {
            break Err(coded(
                "output-limit",
                "Native command output limit exceeded",
            ));
        }
        if Instant::now() >= deadline {
            break Err(coded(
                "native-timeout",
                "Native command time limit exceeded",
            ));
        }
        match child.try_wait() {
            Ok(Some(code)) => break Ok(code),
            // Short commands are common: noticing the exit late should cost
            // at most a twentieth of the wait. The readers wake this thread
            // when the output ends, which is usually the exit.
            Ok(None) => thread::park_timeout(
                (started.elapsed() / 20)
                    .clamp(Duration::from_micros(100), Duration::from_millis(5)),
            ),
            Err(error) => break Err(error),
        }
    };
    // Windows confirms that the owned Job and its guardian have stopped. Unix
    // readers/writers are nonblocking so inherited escaped pipes cannot suspend
    // our bounded cleanup or the caller's deadline indefinitely.
    let cleanup = child.terminate();
    stop.store(true, Ordering::Release);
    // Windows cleanup uncertainty poisons host admission. Do not block joining
    // a pipe whose writer may still exist after an unconfirmed termination.
    cleanup?;
    let stdout = out
        .join()
        .map_err(|_| RuntimeError::unavailable("Native stdout worker failed"))??;
    let stderr = err
        .join()
        .map_err(|_| RuntimeError::unavailable("Native stderr worker failed"))??;
    let input_result = writer
        .map(|writer| {
            writer
                .join()
                .map_err(|_| RuntimeError::unavailable("Native input worker failed"))
        })
        .transpose()?;
    let code = completed?;
    if stdout.len() > limit || stderr.len() > limit {
        return Err(coded(
            "output-limit",
            "Native command output limit exceeded",
        ));
    }
    if input_result.is_some_and(|result| result.is_err()) && code == 0 {
        return Err(RuntimeError::unavailable(
            "Native command did not consume its complete input",
        ));
    }
    Ok(ProcessOutput {
        code,
        stdout,
        stderr,
    })
}

#[cfg(windows)]
pub(crate) fn complete_environment(
    environment: &BTreeMap<OsString, Option<OsString>>,
) -> Result<BTreeMap<String, String>> {
    let mut complete = BTreeMap::<String, String>::new();
    for (key, value) in std::env::vars_os() {
        let key = key
            .into_string()
            .map_err(|_| RuntimeError::invalid("Native environment key is not Unicode"))?;
        let value = value
            .into_string()
            .map_err(|_| RuntimeError::invalid("Native environment value is not Unicode"))?;
        complete.insert(key, value);
    }
    for (key, value) in environment {
        let key = key
            .to_str()
            .ok_or_else(|| RuntimeError::invalid("Native environment key is not Unicode"))?;
        // Windows environment names are case-insensitive. Remove the inherited
        // spelling before inserting an override, including explicit removals.
        let stale = complete
            .keys()
            .filter(|candidate| candidate.eq_ignore_ascii_case(key))
            .cloned()
            .collect::<Vec<_>>();
        for candidate in stale {
            complete.remove(&candidate);
        }
        if let Some(value) = value {
            complete.insert(
                key.into(),
                value
                    .to_str()
                    .ok_or_else(|| {
                        RuntimeError::invalid("Native environment value is not Unicode")
                    })?
                    .into(),
            );
        }
    }

    Ok(complete)
}
/// Interactive native Windows launches inherit the console and stdio while
/// retaining the same Job/guardian custody. Their lifetime is user-controlled.
#[cfg(windows)]
pub fn run_inherited(
    program: &OsStr,
    args: &[OsString],
    cwd: &Path,
    environment: &BTreeMap<OsString, Option<OsString>>,
) -> Result<i32> {
    let complete = complete_environment(environment)?;
    let mut argv = vec![
        program
            .to_str()
            .ok_or_else(|| RuntimeError::invalid("Native executable is not Unicode"))?
            .to_string(),
    ];
    for argument in args {
        argv.push(
            argument
                .to_str()
                .ok_or_else(|| RuntimeError::invalid("Native argument is not Unicode"))?
                .into(),
        );
    }
    let mut child = infra_host::process::GuardedChild::spawn_inherited(&argv, cwd, &complete, None)
        .map_err(host_error)?;
    match child.wait() {
        Ok(code) => Ok(code),
        Err(error) => {
            child
                .kill_and_wait(Duration::from_secs(3))
                .map_err(host_error)?;
            Err(host_error(error))
        }
    }
}
