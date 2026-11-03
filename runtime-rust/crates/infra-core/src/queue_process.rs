//! Owned native worker processes. A worker never passes through a shell.
use crate::{Result, RuntimeError};
use std::{
    ffi::OsString,
    fs::File,
    io::{Read, Write},
    path::Path,
    thread,
    time::{Duration, Instant},
};
pub struct Output {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub _stderr: Vec<u8>,
}
pub fn run(
    engine: &Path,
    args: &[OsString],
    project: &Path,
    input: Vec<u8>,
    timeout: Duration,
    limit: usize,
    environment: &std::collections::BTreeMap<OsString, OsString>,
) -> Result<Output> {
    let mut child = native::spawn(engine, args, project, environment)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| RuntimeError::unavailable("Native queue worker stdout unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| RuntimeError::unavailable("Native queue worker stderr unavailable"))?;
    let stdin = child.stdin.take();
    let out = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let err = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let writer = thread::spawn(move || -> std::io::Result<()> {
        if let Some(mut stdin) = stdin {
            stdin.write_all(&input)?;
        }
        Ok(())
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.poll() {
            Ok(Some(code)) => break Some(code),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            _ => break None,
        }
    };
    // Descendants can retain pipe handles after the leader returned. End the
    // owned group/job before joining any pipe worker, on every completion path.
    child.kill_tree();
    child.reap();
    let input_result = writer.join();
    let stdout = out.join();
    let stderr = err.join();
    let Some(code) = status else {
        return Err(RuntimeError::coded(
            crate::ErrorKind::Unavailable,
            "queue-worker-unfinished",
            "Native queue worker did not finish",
        ));
    };
    let stdout =
        stdout.map_err(|_| RuntimeError::unavailable("Native queue worker stdout failed"))??;
    let stderr =
        stderr.map_err(|_| RuntimeError::unavailable("Native queue worker stderr failed"))??;
    if stdout.len() > limit || stderr.len() > limit {
        return Err(RuntimeError::coded(
            crate::ErrorKind::Unavailable,
            "queue-worker-unfinished",
            "Native queue worker exceeded its output limit",
        ));
    }
    if code == 0
        && input_result
            .map_err(|_| RuntimeError::unavailable("Native queue worker input failed"))?
            .is_err()
    {
        return Err(RuntimeError::coded(
            crate::ErrorKind::Unavailable,
            "queue-worker-unfinished",
            "Native queue worker did not consume its input",
        ));
    }
    Ok(Output {
        code,
        stdout,
        _stderr: stderr,
    })
}
#[cfg(unix)]
mod native {
    use super::*;
    use std::{
        os::{
            fd::{FromRawFd, IntoRawFd},
            unix::process::CommandExt,
        },
        process::{Child as Process, Command, Stdio},
    };
    pub struct Child {
        process: Process,
        pub stdin: Option<File>,
        pub stdout: Option<File>,
        pub stderr: Option<File>,
        group: i32,
    }
    pub fn spawn(
        engine: &Path,
        args: &[OsString],
        project: &Path,
        extra: &std::collections::BTreeMap<OsString, OsString>,
    ) -> Result<Child> {
        let mut command = Command::new(engine);
        command
            .args(args)
            .current_dir(project)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .env("CONTEXT_QUEUE_DRAIN", "1")
            .env("CLAUDE_PROJECT_DIR", project);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("GIT_") {
                command.env_remove(key);
            }
        }
        for (name, value) in extra {
            command.env(name, value);
        }
        let mut process = command.spawn().map_err(|_| {
            RuntimeError::coded(
                crate::ErrorKind::Unavailable,
                "queue-worker-unavailable",
                "Native queue worker could not start",
            )
        })?;
        let group = process.id() as i32;
        let stdin = process
            .stdin
            .take()
            .map(|pipe| unsafe { File::from_raw_fd(pipe.into_raw_fd()) });
        let stdout = process
            .stdout
            .take()
            .map(|pipe| unsafe { File::from_raw_fd(pipe.into_raw_fd()) });
        let stderr = process
            .stderr
            .take()
            .map(|pipe| unsafe { File::from_raw_fd(pipe.into_raw_fd()) });
        Ok(Child {
            process,
            stdin,
            stdout,
            stderr,
            group,
        })
    }
    impl Child {
        pub fn poll(&mut self) -> std::io::Result<Option<i32>> {
            self.process.try_wait().map(|status| {
                status.map(|status| {
                    use std::os::unix::process::ExitStatusExt;
                    status
                        .code()
                        .unwrap_or_else(|| -status.signal().unwrap_or(1))
                })
            })
        }
        pub fn kill_tree(&mut self) {
            unsafe {
                libc::kill(-self.group, libc::SIGKILL);
            }
            let _ = self.process.kill();
        }
        pub fn reap(&mut self) {
            let _ = self.process.wait();
        }
    }
    impl Drop for Child {
        fn drop(&mut self) {
            self.kill_tree();
            self.reap();
        }
    }
}
#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        ffi::c_void,
        io,
        os::windows::{
            ffi::OsStrExt,
            io::{FromRawHandle, RawHandle},
        },
    };
    type Handle = *mut c_void;
    #[repr(C)]
    struct Security {
        length: u32,
        descriptor: *mut c_void,
        inherit: i32,
    }
    #[repr(C)]
    struct Startup {
        cb: u32,
        reserved: *mut u16,
        desktop: *mut u16,
        title: *mut u16,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        chars_x: u32,
        chars_y: u32,
        fill: u32,
        flags: u32,
        show: u16,
        reserved_length: u16,
        reserved_bytes: *mut u8,
        input: Handle,
        output: Handle,
        error: Handle,
    }
    #[repr(C)]
    struct StartupEx {
        base: Startup,
        attributes: *mut c_void,
    }
    #[repr(C)]
    struct ProcessInfo {
        process: Handle,
        thread: Handle,
        pid: u32,
        tid: u32,
    }
    #[repr(C)]
    struct BasicLimit {
        process_time: i64,
        job_time: i64,
        flags: u32,
        min: usize,
        max: usize,
        active: u32,
        affinity: usize,
        priority: u32,
        scheduling: u32,
    }
    #[repr(C)]
    struct IoCounters {
        read_ops: u64,
        write_ops: u64,
        other_ops: u64,
        read_bytes: u64,
        write_bytes: u64,
        other_bytes: u64,
    }
    #[repr(C)]
    struct ExtendedLimit {
        basic: BasicLimit,
        io: IoCounters,
        process: usize,
        job: usize,
        peak_process: usize,
        peak_job: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreatePipe(
            read: *mut Handle,
            write: *mut Handle,
            security: *const Security,
            size: u32,
        ) -> i32;
        fn SetHandleInformation(handle: Handle, mask: u32, flags: u32) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
        fn CreateJobObjectW(security: *const Security, name: *const u16) -> Handle;
        fn SetInformationJobObject(
            job: Handle,
            class: i32,
            value: *const c_void,
            length: u32,
        ) -> i32;
        fn TerminateJobObject(job: Handle, code: u32) -> i32;
        fn InitializeProcThreadAttributeList(
            list: *mut c_void,
            count: u32,
            flags: u32,
            size: *mut usize,
        ) -> i32;
        fn UpdateProcThreadAttribute(
            list: *mut c_void,
            flags: u32,
            attribute: usize,
            value: *mut c_void,
            size: usize,
            previous: *mut c_void,
            returned: *mut usize,
        ) -> i32;
        fn DeleteProcThreadAttributeList(list: *mut c_void);
        fn CreateProcessW(
            application: *const u16,
            line: *mut u16,
            process_security: *const Security,
            thread_security: *const Security,
            inherit: i32,
            flags: u32,
            environment: *const c_void,
            directory: *const u16,
            startup: *mut Startup,
            information: *mut ProcessInfo,
        ) -> i32;
        fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
        fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
        fn TerminateProcess(process: Handle, code: u32) -> i32;
    }
    struct Owned(Handle);
    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }
    fn failure() -> RuntimeError {
        RuntimeError::coded(
            crate::ErrorKind::Unavailable,
            "queue-worker-unavailable",
            "Native queue worker custody unavailable",
        )
    }
    fn admitted(result: i32) -> Result<()> {
        if result == 0 { Err(failure()) } else { Ok(()) }
    }
    fn pipe() -> Result<(Owned, Owned)> {
        let mut read = std::ptr::null_mut();
        let mut write = std::ptr::null_mut();
        let security = Security {
            length: std::mem::size_of::<Security>() as u32,
            descriptor: std::ptr::null_mut(),
            inherit: 1,
        };
        admitted(unsafe { CreatePipe(&mut read, &mut write, &security, 0) })?;
        Ok((Owned(read), Owned(write)))
    }
    fn into_file(mut owned: Owned) -> File {
        let handle = owned.0;
        owned.0 = std::ptr::null_mut();
        unsafe { File::from_raw_handle(handle as RawHandle) }
    }
    fn wide(value: &std::ffi::OsStr) -> Result<Vec<u16>> {
        let mut result = value.encode_wide().collect::<Vec<_>>();
        if result.contains(&0) {
            return Err(RuntimeError::invalid("NUL in native worker launch"));
        }
        result.push(0);
        Ok(result)
    }
    fn quote(value: &std::ffi::OsStr) -> Result<Vec<u16>> {
        let units = value.encode_wide().collect::<Vec<_>>();
        if units.contains(&0) {
            return Err(RuntimeError::invalid("NUL in native worker launch"));
        }
        let mut result = vec![34];
        let mut slashes = 0;
        for unit in units {
            if unit == 92 {
                slashes += 1;
                continue;
            }
            if unit == 34 {
                result.extend(std::iter::repeat_n(92, slashes * 2 + 1));
                result.push(34);
            } else {
                result.extend(std::iter::repeat_n(92, slashes));
                result.push(unit);
            }
            slashes = 0;
        }
        result.extend(std::iter::repeat_n(92, slashes * 2));
        result.push(34);
        Ok(result)
    }
    struct Attributes {
        allocation: Vec<usize>,
    }
    impl Attributes {
        fn new() -> Result<Self> {
            let mut size = 0;
            unsafe {
                InitializeProcThreadAttributeList(std::ptr::null_mut(), 2, 0, &mut size);
            }
            if size == 0 {
                return Err(failure());
            }
            let mut allocation = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
            admitted(unsafe {
                InitializeProcThreadAttributeList(allocation.as_mut_ptr().cast(), 2, 0, &mut size)
            })?;
            Ok(Self { allocation })
        }
        fn pointer(&mut self) -> *mut c_void {
            self.allocation.as_mut_ptr().cast()
        }
        fn add(&mut self, attribute: usize, value: *mut c_void, size: usize) -> Result<()> {
            admitted(unsafe {
                UpdateProcThreadAttribute(
                    self.pointer(),
                    0,
                    attribute,
                    value,
                    size,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            })
        }
    }
    impl Drop for Attributes {
        fn drop(&mut self) {
            unsafe {
                DeleteProcThreadAttributeList(self.pointer());
            }
        }
    }
    pub struct Child {
        process: Owned,
        job: Owned,
        pub stdin: Option<File>,
        pub stdout: Option<File>,
        pub stderr: Option<File>,
    }
    pub fn spawn(
        engine: &Path,
        args: &[OsString],
        project: &Path,
        extra: &std::collections::BTreeMap<OsString, OsString>,
    ) -> Result<Child> {
        let (input_read, input_write) = pipe()?;
        let (output_read, output_write) = pipe()?;
        let (error_read, error_write) = pipe()?;
        for handle in [&input_write, &output_read, &error_read] {
            admitted(unsafe { SetHandleInformation(handle.0, 1, 0) })?;
        }
        let job = Owned(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) });
        if job.0.is_null() {
            return Err(failure());
        }
        let mut limits: ExtendedLimit = unsafe { std::mem::zeroed() };
        limits.basic.flags = 0x2000;
        admitted(unsafe {
            SetInformationJobObject(
                job.0,
                9,
                (&limits as *const ExtendedLimit).cast(),
                std::mem::size_of::<ExtendedLimit>() as u32,
            )
        })?;
        let mut handles = [input_read.0, output_write.0, error_write.0];
        let mut jobs = [job.0];
        let mut attributes = Attributes::new()?;
        attributes.add(
            0x20002,
            handles.as_mut_ptr().cast(),
            std::mem::size_of_val(&handles),
        )?;
        attributes.add(
            0x2000D,
            jobs.as_mut_ptr().cast(),
            std::mem::size_of_val(&jobs),
        )?;
        let application = wide(engine.as_os_str())?;
        let directory = wide(project.as_os_str())?;
        let mut line = quote(engine.as_os_str())?;
        for arg in args {
            line.push(32);
            line.extend(quote(arg)?);
        }
        line.push(0);
        let mut environment = std::env::vars_os()
            .filter(|(name, _)| {
                !name.to_string_lossy().starts_with("GIT_")
                    && !name
                        .to_string_lossy()
                        .eq_ignore_ascii_case("CONTEXT_QUEUE_DRAIN")
                    && !name
                        .to_string_lossy()
                        .eq_ignore_ascii_case("CLAUDE_PROJECT_DIR")
                    && !extra.keys().any(|key| {
                        key.to_string_lossy()
                            .eq_ignore_ascii_case(&name.to_string_lossy())
                    })
            })
            .collect::<Vec<_>>();
        environment.push(("CONTEXT_QUEUE_DRAIN".into(), "1".into()));
        environment.push((
            "CLAUDE_PROJECT_DIR".into(),
            project.as_os_str().to_os_string(),
        ));
        environment.extend(
            extra
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        environment.sort_by(|left, right| {
            left.0
                .to_string_lossy()
                .to_uppercase()
                .cmp(&right.0.to_string_lossy().to_uppercase())
        });
        let mut block = Vec::<u16>::new();
        for (name, value) in environment {
            let mut entry = name.encode_wide().collect::<Vec<_>>();
            entry.push(61);
            entry.extend(value.encode_wide());
            if entry.contains(&0) {
                return Err(failure());
            }
            block.extend(entry);
            block.push(0);
        }
        block.push(0);
        let mut startup: StartupEx = unsafe { std::mem::zeroed() };
        startup.base.cb = std::mem::size_of::<StartupEx>() as u32;
        startup.base.flags = 0x100;
        startup.base.input = input_read.0;
        startup.base.output = output_write.0;
        startup.base.error = error_write.0;
        startup.attributes = attributes.pointer();
        let mut information: ProcessInfo = unsafe { std::mem::zeroed() };
        admitted(unsafe {
            CreateProcessW(
                application.as_ptr(),
                line.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
                0x00080000 | 0x00000400 | 0x08000000,
                block.as_ptr().cast(),
                directory.as_ptr(),
                &mut startup.base,
                &mut information,
            )
        })?;
        // JOB_LIST assigns the job during process creation; no child runs during
        // a post-spawn job-assignment gap. HANDLE_LIST limits inherited handles.
        let process = Owned(information.process);
        drop(Owned(information.thread));
        drop(input_read);
        drop(output_write);
        drop(error_write);
        Ok(Child {
            process,
            job,
            stdin: Some(into_file(input_write)),
            stdout: Some(into_file(output_read)),
            stderr: Some(into_file(error_read)),
        })
    }
    impl Child {
        pub fn poll(&mut self) -> io::Result<Option<i32>> {
            match unsafe { WaitForSingleObject(self.process.0, 0) } {
                258 => Ok(None),
                0 => {
                    let mut code = 0;
                    if unsafe { GetExitCodeProcess(self.process.0, &mut code) } == 0 {
                        Err(io::Error::last_os_error())
                    } else {
                        Ok(Some(code as i32))
                    }
                }
                _ => Err(io::Error::last_os_error()),
            }
        }
        pub fn kill_tree(&mut self) {
            unsafe {
                TerminateJobObject(self.job.0, 1);
                TerminateProcess(self.process.0, 1);
            }
        }
        pub fn reap(&mut self) {
            unsafe {
                WaitForSingleObject(self.process.0, u32::MAX);
            }
        }
    }
    impl Drop for Child {
        fn drop(&mut self) {
            self.kill_tree();
            self.reap();
        }
    }
}
#[cfg(not(any(unix, windows)))]
mod native {
    use super::*;
    pub struct Child {
        pub stdin: Option<File>,
        pub stdout: Option<File>,
        pub stderr: Option<File>,
    }
    pub fn spawn(
        _: &Path,
        _: &[OsString],
        _: &Path,
        _: &std::collections::BTreeMap<OsString, OsString>,
    ) -> Result<Child> {
        Err(RuntimeError::unavailable(
            "Native queue worker process custody is unsupported",
        ))
    }
    impl Child {
        pub fn poll(&mut self) -> std::io::Result<Option<i32>> {
            Ok(None)
        }
        pub fn kill_tree(&mut self) {}
        pub fn reap(&mut self) {}
    }
}
