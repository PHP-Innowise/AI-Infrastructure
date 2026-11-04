//! Windows 10+ native process custody. No shell or batch shim fallback.
//! JOB_LIST assigns actual commands atomically at creation; CREATE_SUSPENDED
//! and verified membership precede ResumeThread. Stable Rust, no Nt* resume API.
//! The trusted guardian stays outside the per-run Job to retain runner exclusion
//! until Job accounting proves that every native descendant has stopped.
use super::{Control, Limits, Output, RunObserver};
use crate::{HarnessError, Result};
use serde_json::Value;
use std::{
    cmp::Ordering as CmpOrdering,
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs::File,
    io::{Read, Write},
    mem,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    ptr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Globalization::CompareStringOrdinal,
    Security::SECURITY_ATTRIBUTES,
    System::{
        Console::{
            GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
        },
        JobObjects::*,
        Pipes::{CreatePipe, PeekNamedPipe},
        Threading::*,
    },
};
static STOPPING: AtomicBool = AtomicBool::new(false);
const CLEANUP: Duration = Duration::from_secs(3);
const RECEIPT_MAGIC: [u8; 4] = *b"IH01";
const CAPTURE_RECEIPT_MAGIC: [u8; 4] = *b"IC01";
const INPUT_LIMIT: usize = 65536;
fn unconfirmed() -> HarnessError {
    HarnessError {
        status: 503,
        code: "cleanup-unconfirmed",
        message:
            "Native process cleanup could not be confirmed; stop admission before another run."
                .into(),
    }
}
fn output_error() -> HarnessError {
    HarnessError {
        status: 503,
        code: "output-limit",
        message: "Native output limit reached.".into(),
    }
}
fn timeout_error() -> HarnessError {
    HarnessError {
        status: 504,
        code: "native-timeout",
        message: "Native time limit reached.".into(),
    }
}
fn invalid(reason: impl std::fmt::Display) -> HarnessError {
    HarnessError::invalid(format!("Invalid native Windows process request: {reason}."))
}
fn os_error() -> HarnessError {
    std::io::Error::last_os_error().into()
}
pub fn is_stopping() -> bool {
    STOPPING.load(Ordering::Acquire)
}
struct Handle(OwnedHandle);
impl Handle {
    unsafe fn adopt(raw: HANDLE) -> Self {
        Self(unsafe { OwnedHandle::from_raw_handle(raw) })
    }
    fn raw(&self) -> HANDLE {
        self.0.as_raw_handle()
    }
    fn file(self) -> File {
        self.0.into()
    }
    fn duplicate(raw: HANDLE, inherit: bool) -> Result<Self> {
        let mut out = ptr::null_mut();
        let process = unsafe { GetCurrentProcess() };
        if unsafe {
            DuplicateHandle(
                process,
                raw,
                process,
                &mut out,
                0,
                i32::from(inherit),
                DUPLICATE_SAME_ACCESS,
            )
        } == 0
        {
            return Err(os_error());
        }
        Ok(unsafe { Self::adopt(out) })
    }
}
fn inheritable(raw: HANDLE, yes: bool) -> Result<()> {
    if unsafe {
        SetHandleInformation(
            raw,
            HANDLE_FLAG_INHERIT,
            if yes { HANDLE_FLAG_INHERIT } else { 0 },
        )
    } == 0
    {
        Err(os_error())
    } else {
        Ok(())
    }
}
fn pipe() -> Result<(Handle, Handle)> {
    let (mut read, mut write) = (ptr::null_mut(), ptr::null_mut());
    let security = SECURITY_ATTRIBUTES {
        nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: 1,
    };
    if unsafe { CreatePipe(&mut read, &mut write, &security, 0) } == 0 {
        return Err(os_error());
    }
    Ok((unsafe { Handle::adopt(read) }, unsafe {
        Handle::adopt(write)
    }))
}
fn wide(value: &OsStr) -> Result<Vec<u16>> {
    let mut out = value.encode_wide().collect::<Vec<_>>();
    if out.contains(&0) {
        return Err(invalid("a string contains NUL"));
    }
    out.push(0);
    Ok(out)
}
fn wide_path(value: &Path) -> Result<Vec<u16>> {
    wide(value.as_os_str())
}
fn ordinal(a: &OsStr, b: &OsStr) -> CmpOrdering {
    let a = a.encode_wide().collect::<Vec<_>>();
    let b = b.encode_wide().collect::<Vec<_>>();
    let value =
        unsafe { CompareStringOrdinal(a.as_ptr(), a.len() as i32, b.as_ptr(), b.len() as i32, 1) };
    match value {
        1 => CmpOrdering::Less,
        3 => CmpOrdering::Greater,
        _ => CmpOrdering::Equal,
    }
}
type Environment = (Vec<u16>, Vec<(OsString, OsString)>);
fn environment(overrides: &BTreeMap<String, String>, remove: &[String]) -> Result<Environment> {
    if remove
        .iter()
        .any(|name| name.is_empty() || name.contains(['=', '\0']))
    {
        return Err(invalid(
            "an environment name to remove is empty or contains = or NUL",
        ));
    }
    let mut pairs = std::env::vars_os()
        .filter(|(key, _)| {
            !remove
                .iter()
                .any(|name| ordinal(key, OsStr::new(name)) == CmpOrdering::Equal)
        })
        .collect::<Vec<_>>();
    for (key, value) in overrides {
        if key.is_empty() || key.contains(['=', '\0']) || value.contains('\0') {
            return Err(invalid("an environment override name or value is invalid"));
        }
        pairs.retain(|(existing, _)| ordinal(existing, OsStr::new(key)) != CmpOrdering::Equal);
        pairs.push((key.into(), value.into()));
    }
    pairs.sort_by(|(a, _), (b, _)| ordinal(a, b));
    let mut block = Vec::new();
    for (key, value) in &pairs {
        let key = key.encode_wide().collect::<Vec<_>>();
        let value = value.encode_wide().collect::<Vec<_>>();
        if key.contains(&0) || value.contains(&0) {
            return Err(invalid("an environment entry contains NUL"));
        }
        block.extend(key);
        block.push('=' as u16);
        block.extend(value);
        block.push(0);
    }
    if block.is_empty() {
        block.push(0)
    }
    block.push(0);
    Ok((block, pairs))
}
fn env_value<'a>(pairs: &'a [(OsString, OsString)], key: &str) -> Option<&'a OsStr> {
    pairs
        .iter()
        .find(|(name, _)| ordinal(name, OsStr::new(key)) == CmpOrdering::Equal)
        .map(|(_, value)| value.as_os_str())
}
fn complete_block(environment: &BTreeMap<String, String>) -> Result<Vec<u16>> {
    let mut pairs = Vec::<(OsString, OsString)>::new();
    for (key, value) in environment {
        // Windows keeps each drive's working directory and the last exit
        // code as `=C:` and `=ExitCode` in inherited environments: a leading
        // `=` belongs to such a name, while any later one would split it.
        let name = key.strip_prefix('=').unwrap_or(key);
        if name.is_empty() || name.contains(['=', '\0']) || value.contains('\0') {
            return Err(invalid(format!("the environment entry {key:?} is invalid")));
        }
        pairs.retain(|(prior, _)| ordinal(prior, OsStr::new(key)) != CmpOrdering::Equal);
        pairs.push((key.into(), value.into()));
    }
    pairs.sort_by(|(a, _), (b, _)| ordinal(a, b));
    let mut block = Vec::new();
    for (key, value) in pairs {
        block.extend(key.encode_wide());
        block.push('=' as u16);
        block.extend(value.encode_wide());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0)
    }
    block.push(0);
    Ok(block)
}
fn native_candidate(path: PathBuf) -> Option<PathBuf> {
    let candidate = if path.extension().is_none() {
        path.with_extension("exe")
    } else {
        path
    };
    let extension = candidate.extension()?.to_str()?;
    if !extension.eq_ignore_ascii_case("exe") && !extension.eq_ignore_ascii_case("com") {
        return None;
    }
    candidate.is_file().then_some(candidate)
}
fn resolve(program: &str, cwd: &Path, pairs: &[(OsString, OsString)]) -> Result<PathBuf> {
    if program.is_empty() || program.contains('\0') {
        return Err(invalid("the program is empty or contains NUL"));
    }
    let p = Path::new(program);
    if p.is_absolute() || program.contains(['\\', '/']) || p.components().count() > 1 {
        let p = if p.is_absolute() {
            p.to_owned()
        } else {
            cwd.join(p)
        };
        return native_candidate(p).ok_or_else(||HarnessError::invalid("Windows command must resolve to a native executable; batch shims require verified provider resolution."));
    }
    let mut directories = vec![
        std::env::current_exe()?.parent().unwrap_or(cwd).to_owned(),
        cwd.to_owned(),
    ];
    if let Some(root) = env_value(pairs, "SystemRoot") {
        directories.push(PathBuf::from(root).join("System32"));
        directories.push(PathBuf::from(root));
    }
    if let Some(path) = env_value(pairs, "PATH") {
        directories.extend(std::env::split_paths(path));
    }
    for directory in directories {
        let directory = if directory.is_absolute() {
            directory
        } else {
            cwd.join(directory)
        };
        if let Some(path) = native_candidate(directory.join(p)) {
            return Ok(path);
        }
    }
    Err(HarnessError::unavailable(
        "Native Windows executable was not found.",
    ))
}
fn quote(argument: &OsStr) -> Result<Vec<u16>> {
    let values = argument.encode_wide().collect::<Vec<_>>();
    if values.contains(&0) {
        return Err(invalid("an argument contains NUL"));
    }
    let mut out = vec!['"' as u16];
    let mut slashes = 0usize;
    for c in values {
        if c == '\\' as u16 {
            slashes += 1;
            continue;
        }
        if c == '"' as u16 {
            out.extend(std::iter::repeat_n('\\' as u16, slashes * 2 + 1));
        } else {
            out.extend(std::iter::repeat_n('\\' as u16, slashes));
        }
        slashes = 0;
        out.push(c);
    }
    out.extend(std::iter::repeat_n('\\' as u16, slashes * 2));
    out.push('"' as u16);
    Ok(out)
}
struct Attributes {
    storage: Vec<usize>,
    list: LPPROC_THREAD_ATTRIBUTE_LIST,
    jobs: Vec<HANDLE>,
}
impl Attributes {
    fn handles(handles: &[HANDLE], job: Option<&Handle>) -> Result<Self> {
        let count = if job.is_some() { 2 } else { 1 };
        let mut bytes = 0usize;
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), count, 0, &mut bytes) };
        if bytes == 0 || bytes > 1024 * 1024 {
            return Err(os_error());
        }
        let mut storage = vec![0usize; bytes.div_ceil(mem::size_of::<usize>())];
        let list = storage.as_mut_ptr().cast();
        if unsafe { InitializeProcThreadAttributeList(list, count, 0, &mut bytes) } == 0 {
            return Err(os_error());
        }
        let jobs = job.map(|j| vec![j.raw()]).unwrap_or_default();
        let out = Self {
            storage,
            list,
            jobs,
        };
        if unsafe {
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast(),
                mem::size_of_val(handles),
                ptr::null_mut(),
                ptr::null(),
            )
        } == 0
        {
            return Err(os_error());
        }
        if !out.jobs.is_empty()
            && unsafe {
                UpdateProcThreadAttribute(
                    list,
                    0,
                    PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
                    out.jobs.as_ptr().cast(),
                    mem::size_of_val(out.jobs.as_slice()),
                    ptr::null_mut(),
                    ptr::null(),
                )
            } == 0
        {
            return Err(os_error());
        }
        Ok(out)
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.list) };
        let _ = &self.storage;
    }
}
struct Native {
    process: Handle,
}
fn exited(process: &Handle) -> Result<bool> {
    match unsafe { WaitForSingleObject(process.raw(), 0) } {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        _ => Err(os_error()),
    }
}
fn job_empty(job: &Handle) -> Result<bool> {
    let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { mem::zeroed() };
    if unsafe {
        QueryInformationJobObject(
            job.raw(),
            JobObjectBasicAccountingInformation,
            (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
            mem::size_of_val(&info) as u32,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(os_error());
    }
    Ok(info.ActiveProcesses == 0)
}
fn new_job() -> Result<Handle> {
    let raw = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if raw.is_null() {
        return Err(os_error());
    }
    let job = unsafe { Handle::adopt(raw) };
    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { mem::zeroed() };
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if unsafe {
        SetInformationJobObject(
            job.raw(),
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            mem::size_of_val(&info) as u32,
        )
    } == 0
    {
        return Err(os_error());
    }
    Ok(job)
}
#[expect(
    clippy::too_many_arguments,
    reason = "The Windows launch boundary keeps argv, environment, handle inheritance and Job controls explicit."
)]
fn create(
    executable: &Path,
    arguments: &[OsString],
    cwd: &Path,
    environment: &[u16],
    stdio: [HANDLE; 3],
    extra: &[HANDLE],
    job: Option<&Handle>,
    new_group: bool,
) -> Result<Native> {
    let app = wide_path(executable)?;
    let directory = wide_path(cwd)?;
    let mut command = quote(executable.as_os_str())?;
    for argument in arguments {
        command.push(' ' as u16);
        command.extend(quote(argument)?);
    }
    command.push(0);
    if command.len() > 32767 {
        return Err(invalid("the command line exceeds 32767 characters"));
    }
    let mut handles = stdio.to_vec();
    handles.extend_from_slice(extra);
    handles.sort_by_key(|h| *h as usize);
    handles.dedup();
    if handles
        .iter()
        .any(|h| h.is_null() || *h == INVALID_HANDLE_VALUE)
    {
        return Err(invalid("a standard handle is unavailable"));
    }
    let attributes = Attributes::handles(&handles, job)?;
    let mut start: STARTUPINFOEXW = unsafe { mem::zeroed() };
    start.StartupInfo.cb = mem::size_of::<STARTUPINFOEXW>() as u32;
    start.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    start.StartupInfo.hStdInput = stdio[0];
    start.StartupInfo.hStdOutput = stdio[1];
    start.StartupInfo.hStdError = stdio[2];
    start.lpAttributeList = attributes.list;
    let mut info: PROCESS_INFORMATION = unsafe { mem::zeroed() };
    let flags = EXTENDED_STARTUPINFO_PRESENT
        | CREATE_UNICODE_ENVIRONMENT
        | if new_group {
            CREATE_NEW_PROCESS_GROUP
        } else {
            0
        }
        | if job.is_some() { CREATE_SUSPENDED } else { 0 };
    if unsafe {
        CreateProcessW(
            app.as_ptr(),
            command.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            flags,
            environment.as_ptr().cast(),
            directory.as_ptr(),
            &start.StartupInfo,
            &mut info,
        )
    } == 0
    {
        return Err(os_error());
    }
    let process = unsafe { Handle::adopt(info.hProcess) };
    let thread = unsafe { Handle::adopt(info.hThread) };
    if let Some(job) = job {
        let mut assigned = 0;
        if unsafe { IsProcessInJob(process.raw(), job.raw(), &mut assigned) } == 0 || assigned == 0
        {
            let failure = os_error();
            unsafe { TerminateProcess(process.raw(), 1) };
            if unsafe { WaitForSingleObject(process.raw(), 3000) } != WAIT_OBJECT_0 {
                return Err(unconfirmed());
            }
            return Err(failure);
        }
        if unsafe { ResumeThread(thread.raw()) } == u32::MAX {
            let failure = os_error();
            unsafe { TerminateJobObject(job.raw(), 1) };
            return Err(failure);
        }
    }
    drop(thread);
    Ok(Native { process })
}
fn kill_job(job: &Handle) {
    unsafe { TerminateJobObject(job.raw(), 1) };
}
struct Owner {
    job: Option<Handle>,
    guardian: Option<Handle>,
    watch: Option<File>,
    completion: Option<File>,
    runner: Option<Arc<File>>,
    settled: bool,
}
impl Owner {
    fn stop(&mut self) {
        self.watch.take();
        if let Some(job) = &self.job {
            kill_job(job)
        }
    }
    fn ready(&self) -> bool {
        self.job
            .as_ref()
            .is_some_and(|job| job_empty(job).unwrap_or(false))
            && self
                .guardian
                .as_ref()
                .is_some_and(|p| exited(p).unwrap_or(false))
    }
    fn packet(&mut self) -> Result<(u32, i32)> {
        let completion = self.completion.as_mut().ok_or_else(unconfirmed)?;
        let mut packet = [0u8; 13];
        let mut used = 0;
        while used < packet.len() {
            match completion.read(&mut packet[used..]) {
                Ok(0) => break,
                Ok(n) => used += n,
                Err(e) => return Err(e.into()),
            }
        }
        if used != 12 || packet[..4] != RECEIPT_MAGIC {
            return Err(HarnessError::unavailable(format!(
                "Native guardian did not return a valid cleanup receipt ({used} of 12 bytes)."
            )));
        }
        let kind = u32::from_le_bytes(packet[4..8].try_into().unwrap());
        let code = i32::from_le_bytes(packet[8..12].try_into().unwrap());
        if kind > 2 {
            return Err(HarnessError::unavailable(
                "Native guardian returned an invalid cleanup receipt.",
            ));
        }
        Ok((kind, code))
    }
    async fn cleanup(&mut self) -> Result<(u32, i32)> {
        self.stop();
        let deadline = Instant::now() + CLEANUP;
        loop {
            if self.ready() {
                self.settled = true;
                return self.packet();
            }
            if Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if self
            .job
            .as_ref()
            .is_some_and(|job| job_empty(job).unwrap_or(false))
            && let Some(guardian) = &self.guardian
        {
            unsafe { TerminateProcess(guardian.raw(), 1) };
            let mut attempts = 100;
            while attempts > 0 && !exited(guardian).unwrap_or(false) {
                tokio::time::sleep(Duration::from_millis(10)).await;
                attempts -= 1
            }
            if exited(guardian).unwrap_or(false) {
                self.settled = true;
                return Err(HarnessError::unavailable(
                    "Native guardian stopped without a cleanup receipt.",
                ));
            }
        }
        STOPPING.store(true, Ordering::Release);
        Err(unconfirmed())
    }
    fn recover(&mut self) {
        STOPPING.store(true, Ordering::Release);
        struct Recovery {
            job: Option<Handle>,
            guardian: Option<Handle>,
            runner: Option<Arc<File>>,
            completion: Option<File>,
        }
        let custody = Arc::new(Recovery {
            job: self.job.take(),
            guardian: self.guardian.take(),
            runner: self.runner.take(),
            completion: self.completion.take(),
        });
        let worker = custody.clone();
        let task = std::thread::Builder::new()
            .name("infra-custody-recovery".into())
            .spawn(move || {
                let _ = &worker.runner;
                let _ = &worker.completion;
                loop {
                    if let Some(job) = &worker.job {
                        kill_job(job);
                        if job_empty(job).unwrap_or(false) {
                            if let Some(guardian) = &worker.guardian {
                                unsafe { TerminateProcess(guardian.raw(), 1) };
                                if !exited(guardian).unwrap_or(false) {
                                    std::thread::sleep(Duration::from_millis(50));
                                    continue;
                                }
                            }
                            break;
                        }
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            });
        if task.is_err() {
            mem::forget(custody)
        }
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        self.stop();
        let deadline = Instant::now() + CLEANUP;
        while Instant::now() < deadline {
            if self.ready() {
                self.settled = true;
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        self.recover();
    }
}

/// Synchronous pipe owner for repository tools and persistent RPC clients.
/// spawn receives the COMPLETE child environment; absent keys stay absent.
/// The provider-facing NativeSession merges inherited environment separately.
pub struct GuardedChild {
    pub stdin: Option<File>,
    pub stdout: Option<File>,
    pub stderr: Option<File>,
    owner: Owner,
    started: Instant,
    exit_code: Option<i32>,
}
impl GuardedChild {
    pub fn spawn(
        argv: &[String],
        cwd: &Path,
        complete_environment: &BTreeMap<String, String>,
        runner_lock: Option<Arc<File>>,
        capture_stderr: bool,
    ) -> Result<Self> {
        spawn_prepared(
            argv,
            cwd,
            &complete_block(complete_environment)?,
            runner_lock,
            capture_stderr,
            false,
            false,
        )
    }
    pub fn spawn_inherited(
        argv: &[String],
        cwd: &Path,
        complete_environment: &BTreeMap<String, String>,
        runner_lock: Option<Arc<File>>,
    ) -> Result<Self> {
        spawn_prepared(
            argv,
            cwd,
            &complete_block(complete_environment)?,
            runner_lock,
            false,
            true,
            false,
        )
    }
    pub fn id(&self) -> u32 {
        self.owner
            .guardian
            .as_ref()
            .map(|h| unsafe { GetProcessId(h.raw()) })
            .unwrap_or(0)
    }
    pub fn as_raw_handle(&self) -> std::os::windows::io::RawHandle {
        self.owner
            .guardian
            .as_ref()
            .map(|h| h.raw())
            .unwrap_or(ptr::null_mut())
    }
    pub fn try_wait(&mut self) -> Result<Option<i32>> {
        if let Some(code) = self.exit_code {
            return Ok(Some(code));
        }
        if !self.owner.ready() {
            return Ok(None);
        }
        self.owner.settled = true;
        let (kind, code) = self.owner.packet()?;
        if kind == 1 {
            return Err(HarnessError::unavailable(
                "Native guardian could not launch the command.",
            ));
        }
        self.exit_code = Some(code);
        Ok(Some(code))
    }
    pub fn wait(&mut self) -> Result<i32> {
        loop {
            if let Some(code) = self.try_wait()? {
                return Ok(code);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    pub fn kill_and_wait(&mut self, timeout: Duration) -> Result<i32> {
        self.stdin.take();
        self.owner.stop();
        if let Some(code) = self.exit_code {
            return Ok(code);
        }
        let until = Instant::now() + timeout;
        loop {
            if self.owner.ready() {
                self.owner.settled = true;
                let code = self.owner.packet().map(|(_, c)| c).unwrap_or(1);
                self.exit_code = Some(code);
                return Ok(code);
            }
            if Instant::now() >= until {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if self
            .owner
            .job
            .as_ref()
            .is_some_and(|job| job_empty(job).unwrap_or(false))
            && let Some(guardian) = &self.owner.guardian
        {
            unsafe { TerminateProcess(guardian.raw(), 1) };
            if unsafe { WaitForSingleObject(guardian.raw(), 1000) } == WAIT_OBJECT_0 {
                self.owner.settled = true;
                self.exit_code = Some(1);
                return Ok(1);
            }
        }
        STOPPING.store(true, Ordering::Release);
        Err(unconfirmed())
    }
}
struct StdioChannels {
    child: [Handle; 3],
    input: Option<File>,
    output: Option<File>,
    error: Option<File>,
}
fn stdio_handle(which: u32) -> Result<Handle> {
    let raw = unsafe { GetStdHandle(which) };
    if raw.is_null() || raw == INVALID_HANDLE_VALUE {
        let null = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("NUL")?;
        Handle::duplicate(null.as_raw_handle(), true)
    } else {
        Handle::duplicate(raw, true)
    }
}
fn stdio_channels(capture: bool, inherited: bool, merged: bool) -> Result<StdioChannels> {
    if inherited {
        return Ok(StdioChannels {
            child: [
                stdio_handle(STD_INPUT_HANDLE)?,
                stdio_handle(STD_OUTPUT_HANDLE)?,
                stdio_handle(STD_ERROR_HANDLE)?,
            ],
            input: None,
            output: None,
            error: None,
        });
    }
    let (stdin_read, stdin_write) = pipe()?;
    inheritable(stdin_write.raw(), false)?;
    let (stdout_read, stdout_write) = pipe()?;
    inheritable(stdout_read.raw(), false)?;
    let (error_read, error_write) = if merged {
        (None, Handle::duplicate(stdout_write.raw(), true)?)
    } else if capture {
        let (read, write) = pipe()?;
        inheritable(read.raw(), false)?;
        (Some(read.file()), write)
    } else {
        let null = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("NUL")?;
        (None, Handle::duplicate(null.as_raw_handle(), true)?)
    };
    Ok(StdioChannels {
        child: [stdin_read, stdout_write, error_write],
        input: Some(stdin_write.file()),
        output: Some(stdout_read.file()),
        error: error_read,
    })
}
fn spawn_prepared(
    argv: &[String],
    cwd: &Path,
    env: &[u16],
    runner_lock: Option<Arc<File>>,
    capture_stderr: bool,
    inherited_stdio: bool,
    merge_stderr: bool,
) -> Result<GuardedChild> {
    if is_stopping() {
        return Err(unconfirmed());
    }
    if argv.is_empty() || argv.iter().any(|s| s.contains('\0')) {
        return Err(invalid("the argument vector is empty or contains NUL"));
    }
    let cwd = infra_core::fs::canonicalize(cwd)?;
    if !cwd.is_dir() {
        return Err(invalid(format!(
            "the working directory {} is not a directory",
            cwd.display()
        )));
    }
    let job = new_job()?;
    let job_guard = Handle::duplicate(job.raw(), true)?;
    let (watch_read, watch_write) = pipe()?;
    inheritable(watch_write.raw(), false)?;
    let (completion_read, completion_write) = pipe()?;
    inheritable(completion_read.raw(), false)?;
    let channels = stdio_channels(capture_stderr, inherited_stdio, merge_stderr)?;
    let runner = runner_lock
        .as_ref()
        .map(|f| Handle::duplicate(f.as_raw_handle(), true))
        .transpose()?;
    let executable = super::guardian()?;
    let mut args = vec![
        OsString::from("internal-guard"),
        OsString::from("--watchdog-handle"),
        OsString::from((watch_read.raw() as usize).to_string()),
        OsString::from("--completion-handle"),
        OsString::from((completion_write.raw() as usize).to_string()),
        OsString::from("--job-handle"),
        OsString::from((job_guard.raw() as usize).to_string()),
    ];
    if inherited_stdio {
        args.push("--interactive".into())
    }
    if let Some(lock) = &runner {
        args.push("--runner-lock-handle".into());
        args.push((lock.raw() as usize).to_string().into());
    }
    args.push("--".into());
    args.extend(argv.iter().map(OsString::from));
    let mut extra = vec![watch_read.raw(), completion_write.raw(), job_guard.raw()];
    if let Some(lock) = &runner {
        extra.push(lock.raw());
    }
    let started = Instant::now();
    let child = create(
        &executable,
        &args,
        &cwd,
        env,
        [
            channels.child[0].raw(),
            channels.child[1].raw(),
            channels.child[2].raw(),
        ],
        &extra,
        None,
        !inherited_stdio,
    )?;
    let owner = Owner {
        job: Some(job),
        guardian: Some(child.process),
        watch: Some(watch_write.file()),
        completion: Some(completion_read.file()),
        runner: runner_lock,
        settled: false,
    };
    drop(watch_read);
    drop(completion_write);
    drop(job_guard);
    drop(channels.child);
    drop(runner);
    Ok(GuardedChild {
        stdin: channels.input,
        stdout: channels.output,
        stderr: channels.error,
        owner,
        started,
        exit_code: None,
    })
}

// Each operation and buffer stays at its heap address until Windows reports
// completion. Cancellation is a request; it is never treated as completion.
struct CapturePending {
    overlapped: std::cell::UnsafeCell<windows_sys::Win32::System::IO::OVERLAPPED>,
    bytes: Box<std::cell::UnsafeCell<[u8; 65536]>>,
    _event: Handle,
}
// SAFETY: the kernel alone writes these UnsafeCell regions while I/O is pending.
// Rust reads the buffer only after completion; the single owner transfers the
// whole allocation to recovery before another thread may query it.
unsafe impl Send for CapturePending {}
unsafe impl Sync for CapturePending {}
impl CapturePending {
    fn new() -> Result<Box<Self>> {
        let raw = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
        if raw.is_null() {
            return Err(os_error());
        }
        let event = unsafe { Handle::adopt(raw) };
        let mut overlapped: windows_sys::Win32::System::IO::OVERLAPPED = unsafe { mem::zeroed() };
        overlapped.hEvent = event.raw();
        Ok(Box::new(Self {
            overlapped: std::cell::UnsafeCell::new(overlapped),
            bytes: Box::new(std::cell::UnsafeCell::new([0; 65536])),
            _event: event,
        }))
    }
}
pub(crate) struct CaptureReadPipe {
    handle: Option<Handle>,
    pending: Option<Box<CapturePending>>,
    eof: bool,
}
impl CaptureReadPipe {
    fn raw(&self) -> HANDLE {
        self.handle.as_ref().unwrap().raw()
    }
    pub(crate) fn eof(&self) -> bool {
        self.eof
    }
    fn completed(&self) -> Option<(u32, Option<std::io::Error>)> {
        let pending = self.pending.as_ref()?;
        // Only the private event can prove that Windows has released the
        // operation's addresses; an unrelated API error is not such proof.
        if unsafe { WaitForSingleObject(pending._event.raw(), 0) } != WAIT_OBJECT_0 {
            return None;
        }
        let mut count = 0u32;
        if unsafe {
            windows_sys::Win32::System::IO::GetOverlappedResult(
                self.raw(),
                pending.overlapped.get(),
                &mut count,
                0,
            )
        } != 0
        {
            return Some((count, None));
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_IO_INCOMPLETE as i32) {
            None
        } else {
            Some((0, Some(error)))
        }
    }
    pub(crate) fn poll(&mut self) -> Result<Option<Vec<u8>>> {
        if self.eof {
            return Ok(None);
        }
        if self.pending.is_none() {
            self.pending = Some(CapturePending::new()?);
            let pending = self.pending.as_ref().unwrap();
            let started = unsafe {
                windows_sys::Win32::Storage::FileSystem::ReadFile(
                    self.raw(),
                    pending.bytes.get().cast::<u8>(),
                    65536,
                    ptr::null_mut(),
                    pending.overlapped.get(),
                )
            };
            if started == 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
                    // A nonpending immediate failure retains no kernel request.
                    self.pending.take();
                    if capture_pipe_eof(&error) {
                        self.eof = true;
                        return Ok(None);
                    }
                    return Err(error.into());
                }
            }
        }
        let Some((count, failure)) = self.completed() else {
            return Ok(None);
        };
        let pending = self.pending.take().unwrap();
        if let Some(error) = failure {
            if capture_pipe_eof(&error) {
                self.eof = true;
                return Ok(None);
            }
            return Err(error.into());
        }
        if count == 0 {
            // A zero-length WriteFile can complete a pipe read successfully.
            // It is not EOF; a later operation must still accept later bytes.
            return Ok(None);
        }
        if count > 65536 {
            return Err(invalid("a pipe read reported more bytes than its buffer"));
        }
        // SAFETY: GetOverlappedResult confirmed the operation has completed.
        let bytes = unsafe { &*pending.bytes.get() };
        Ok(Some(bytes[..count as usize].to_vec()))
    }
    fn cancel(&self) {
        if let Some(pending) = &self.pending {
            unsafe {
                windows_sys::Win32::System::IO::CancelIoEx(self.raw(), pending.overlapped.get());
            }
        }
    }
    pub(crate) async fn settle(&mut self) -> Result<()> {
        self.cancel();
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if self.pending.is_none() || self.completed().is_some() {
                self.pending.take();
                self.eof = true;
                return Ok(());
            }
            if Instant::now() >= deadline {
                STOPPING.store(true, Ordering::Release);
                return Err(unconfirmed());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
impl Drop for CaptureReadPipe {
    fn drop(&mut self) {
        if self.pending.is_none() {
            return;
        }
        self.cancel();
        if self.completed().is_some() {
            self.pending.take();
            return;
        }
        STOPPING.store(true, Ordering::Release);
        struct Recovery {
            handle: Handle,
            pending: Box<CapturePending>,
        }
        let custody = Arc::new(Recovery {
            handle: self.handle.take().unwrap(),
            pending: self.pending.take().unwrap(),
        });
        let worker = custody.clone();
        let thread = std::thread::Builder::new()
            .name("infra-capture-io-recovery".into())
            .spawn(move || {
                loop {
                    unsafe {
                        windows_sys::Win32::System::IO::CancelIoEx(
                            worker.handle.raw(),
                            worker.pending.overlapped.get(),
                        );
                    }
                    if unsafe { WaitForSingleObject(worker.pending._event.raw(), 0) }
                        == WAIT_OBJECT_0
                    {
                        let mut count = 0;
                        let completed = unsafe {
                            windows_sys::Win32::System::IO::GetOverlappedResult(
                                worker.handle.raw(),
                                worker.pending.overlapped.get(),
                                &mut count,
                                0,
                            )
                        };
                        if completed != 0
                            || std::io::Error::last_os_error().raw_os_error()
                                != Some(ERROR_IO_INCOMPLETE as i32)
                        {
                            break;
                        }
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            });
        if thread.is_err() {
            // Keeping the allocations and handles is necessary: the kernel may
            // still own their addresses. Admission remains stopped.
            mem::forget(custody);
        }
    }
}
fn capture_pipe_eof(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(value)
        if value == ERROR_BROKEN_PIPE as i32
            || value == ERROR_PIPE_NOT_CONNECTED as i32
            || value == ERROR_NO_DATA as i32)
}
fn capture_pipe() -> Result<(CaptureReadPipe, Handle)> {
    use windows_sys::Win32::{
        Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_FIRST_PIPE_INSTANCE,
            FILE_FLAG_OVERLAPPED, OPEN_EXISTING, PIPE_ACCESS_INBOUND,
        },
        System::Pipes::{
            CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE,
            PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
        },
    };
    let name = wide(OsStr::new(&format!(
        r"\\.\pipe\infra-capture-{}-{}",
        unsafe { GetCurrentProcessId() },
        uuid::Uuid::new_v4(),
    )))?;
    let read = infra_core::security::with_private_creation_descriptor(|descriptor| {
        let security = SECURITY_ATTRIBUTES {
            nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let raw = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_INBOUND | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                65536,
                65536,
                0,
                &security,
            )
        };
        if raw == INVALID_HANDLE_VALUE {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(unsafe { Handle::adopt(raw) })
        }
    })?;
    let security = SECURITY_ATTRIBUTES {
        nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: 1,
    };
    // CreateFile establishes this first connection before any ConnectNamedPipe
    // wait is needed. Verify that the client endpoint belongs to this process.
    let raw = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_WRITE,
            0,
            &security,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(os_error());
    }
    let write = unsafe { Handle::adopt(raw) };
    let mut client = 0u32;
    if unsafe { GetNamedPipeClientProcessId(read.raw(), &mut client) } == 0
        || client != unsafe { GetCurrentProcessId() }
    {
        return Err(invalid("the pipe client is another process"));
    }
    Ok((
        CaptureReadPipe {
            handle: Some(read),
            pending: None,
            eof: false,
        },
        write,
    ))
}

/// Separate native status and delayed cleanup for finite captured executions.
pub(crate) struct CaptureChild {
    pub(crate) stdin: Option<File>,
    pub(crate) stdout: Option<CaptureReadPipe>,
    pub(crate) stderr: Option<CaptureReadPipe>,
    owner: Owner,
    native: CaptureReadPipe,
    packet: Vec<u8>,
    code: Option<i32>,
}
impl CaptureChild {
    pub(crate) fn spawn(
        argv: &[String],
        cwd: &Path,
        complete_environment: &BTreeMap<String, String>,
    ) -> Result<Self> {
        if is_stopping() {
            return Err(unconfirmed());
        }
        if argv.is_empty() || argv.iter().any(|argument| argument.contains('\0')) {
            return Err(invalid("the argument vector is empty or contains NUL"));
        }
        let cwd = infra_core::fs::canonicalize(cwd)?;
        if !cwd.is_dir() {
            return Err(invalid(format!(
                "the working directory {} is not a directory",
                cwd.display()
            )));
        }
        let environment = complete_block(complete_environment)?;
        let job = new_job()?;
        let job_guard = Handle::duplicate(job.raw(), true)?;
        let (watch_read, watch_write) = pipe()?;
        inheritable(watch_write.raw(), false)?;
        let (completion_read, completion_write) = pipe()?;
        inheritable(completion_read.raw(), false)?;
        let (native, native_write) = capture_pipe()?;
        let (stdin_read, stdin_write) = pipe()?;
        inheritable(stdin_write.raw(), false)?;
        let (stdout, stdout_write) = capture_pipe()?;
        let (stderr, stderr_write) = capture_pipe()?;
        let executable = super::guardian()?;
        let mut arguments = vec![
            OsString::from("internal-guard"),
            OsString::from("--watchdog-handle"),
            OsString::from((watch_read.raw() as usize).to_string()),
            OsString::from("--completion-handle"),
            OsString::from((completion_write.raw() as usize).to_string()),
            OsString::from("--job-handle"),
            OsString::from((job_guard.raw() as usize).to_string()),
            OsString::from("--native-exit-handle"),
            OsString::from((native_write.raw() as usize).to_string()),
            OsString::from("--"),
        ];
        arguments.extend(argv.iter().map(OsString::from));
        let guardian = create(
            &executable,
            &arguments,
            &cwd,
            &environment,
            [stdin_read.raw(), stdout_write.raw(), stderr_write.raw()],
            &[
                watch_read.raw(),
                completion_write.raw(),
                job_guard.raw(),
                native_write.raw(),
            ],
            None,
            true,
        )?;
        let owner = Owner {
            job: Some(job),
            guardian: Some(guardian.process),
            watch: Some(watch_write.file()),
            completion: Some(completion_read.file()),
            runner: None,
            settled: false,
        };
        drop(watch_read);
        drop(completion_write);
        drop(job_guard);
        drop(native_write);
        drop(stdin_read);
        drop(stdout_write);
        drop(stderr_write);
        Ok(Self {
            stdin: Some(stdin_write.file()),
            stdout: Some(stdout),
            stderr: Some(stderr),
            owner,
            native,
            packet: Vec::with_capacity(13),
            code: None,
        })
    }
    pub(crate) fn poll_native_exit(&mut self) -> Result<Option<i32>> {
        if let Some(code) = self.code {
            return Ok(Some(code));
        }
        if let Some(bytes) = self.native.poll()? {
            self.packet.extend(bytes);
        }
        if self.packet.len() > 12 {
            return Err(HarnessError::unavailable(
                "Invalid captured native exit receipt.",
            ));
        }
        if !self.native.eof() {
            return Ok(None);
        }
        if self.packet.len() != 12 || self.packet[..4] != CAPTURE_RECEIPT_MAGIC {
            return Err(HarnessError::unavailable(
                "Missing captured native exit receipt.",
            ));
        }
        let kind = u32::from_le_bytes(self.packet[4..8].try_into().unwrap());
        let code = i32::from_le_bytes(self.packet[8..12].try_into().unwrap());
        if kind != 0 {
            return Err(HarnessError::unavailable(
                "Captured native command did not return an exit status.",
            ));
        }
        self.code = Some(code);
        Ok(Some(code))
    }
    pub(crate) async fn finish(&mut self) -> (Result<()>, Result<Option<i32>>) {
        let cleanup = self.owner.cleanup().await.and_then(|(kind, _)| {
            if kind == 1 {
                Err(HarnessError::unavailable(
                    "Native guardian could not complete captured process custody.",
                ))
            } else {
                Ok(())
            }
        });
        // Even a deadline/callback/input stop can race a native exit that the
        // guardian has already observed. Drain its separate validated lane after
        // owned cleanup; never substitute the cleanup receipt's code.
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut observed = loop {
            match self.poll_native_exit() {
                Ok(Some(code)) => break Ok(Some(code)),
                Err(error) => break Err(error),
                Ok(None) => {}
            }
            if Instant::now() >= deadline {
                break Err(HarnessError::unavailable(
                    "Captured native exit observation did not finish.",
                ));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        if let Err(error) = self.native.settle().await {
            observed = Err(error);
        }
        (cleanup, observed)
    }
    pub(crate) fn stop_admission() {
        STOPPING.store(true, Ordering::Release);
    }
}

/// Bounded native Windows pipe session shared by one-shot and RPC callers.
/// Commands are native executable argv, already resolved by provider shim policy.
pub struct NativeSession {
    input: Option<mpsc::SyncSender<Vec<u8>>>,
    owner: Owner,
    stdout: File,
    writer_result: mpsc::Receiver<Result<()>>,
    writer_done: bool,
    started: Instant,
    limits: Limits,
    cancel: Arc<AtomicBool>,
    output: Vec<u8>,
    line: Vec<u8>,
    eof: bool,
    input_used: usize,
    input_limit: Option<usize>,
    finished: bool,
    observer: Option<Arc<dyn RunObserver>>,
    deadline: Option<Instant>,
}
impl NativeSession {
    pub fn spawn(
        argv: &[String],
        cwd: &Path,
        overrides: &BTreeMap<String, String>,
        limits: Limits,
        cancel: Arc<AtomicBool>,
        runner_lock: Option<Arc<File>>,
        input_limit: Option<usize>,
    ) -> Result<Self> {
        Self::spawn_options(
            argv,
            cwd,
            overrides,
            &[],
            limits,
            cancel,
            runner_lock,
            input_limit,
            false,
            None,
        )
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "The Windows session constructor keeps per-call stream options, observer and runner custody explicit."
    )]
    fn spawn_options(
        argv: &[String],
        cwd: &Path,
        overrides: &BTreeMap<String, String>,
        remove: &[String],
        limits: Limits,
        cancel: Arc<AtomicBool>,
        runner_lock: Option<Arc<File>>,
        input_limit: Option<usize>,
        merge_stderr: bool,
        observer: Option<Arc<dyn RunObserver>>,
    ) -> Result<Self> {
        Self::spawn_options_until(
            argv,
            cwd,
            overrides,
            remove,
            limits,
            cancel,
            runner_lock,
            input_limit,
            merge_stderr,
            observer,
            None,
        )
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "The Windows session constructor carries an absolute deadline with per-call stream and custody options."
    )]
    fn spawn_options_until(
        argv: &[String],
        cwd: &Path,
        overrides: &BTreeMap<String, String>,
        remove: &[String],
        limits: Limits,
        cancel: Arc<AtomicBool>,
        runner_lock: Option<Arc<File>>,
        input_limit: Option<usize>,
        merge_stderr: bool,
        observer: Option<Arc<dyn RunObserver>>,
        deadline: Option<Instant>,
    ) -> Result<Self> {
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(timeout_error());
        }
        let (env, _) = environment(overrides, remove)?;
        let mut child = spawn_prepared(argv, cwd, &env, runner_lock, false, false, merge_stderr)?;
        if let Some(observer) = &observer
            && let Err(error) = observer.boundary(
                "spawned",
                &serde_json::json!({"pid":child.id(),"kind":"guardian"}),
            )
        {
            return match child.kill_and_wait(CLEANUP) {
                Err(cleanup) if cleanup.code == "cleanup-unconfirmed" => Err(cleanup),
                _ => Err(error),
            };
        }
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| HarnessError::unavailable("Native stdin pipe is missing."))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| HarnessError::unavailable("Native stdout pipe is missing."))?;
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(8);
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        let writer_observer = observer.clone();
        let mut stdin = stdin;
        std::thread::Builder::new()
            .name("infra-native-stdin".into())
            .spawn(move || {
                let mut result = (|| -> Result<()> {
                    while let Ok(packet) = rx.recv() {
                        match stdin.write_all(&packet) {
                            Ok(()) => {}
                            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => return Ok(()),
                            Err(e) => return Err(e.into()),
                        }
                        stdin.flush()?;
                        if let Some(observer) = &writer_observer {
                            observer.boundary(
                                "input_written",
                                &serde_json::json!({"bytes":packet.len()}),
                            )?;
                        }
                    }
                    Ok(())
                })();
                drop(stdin);
                if let Some(observer) = &writer_observer
                    && let Err(error) = observer.boundary("input_closed", &serde_json::json!({}))
                    && result.is_ok()
                {
                    result = Err(error)
                }
                let _ = done_tx.send(result);
            })?;
        let GuardedChild { owner, started, .. } = child;
        Ok(Self {
            input: Some(tx),
            owner,
            stdout,
            writer_result: done_rx,
            writer_done: false,
            started,
            limits,
            cancel,
            output: Vec::new(),
            line: Vec::new(),
            eof: false,
            input_used: 0,
            input_limit,
            finished: false,
            observer,
            deadline,
        })
    }
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        if self.finished {
            return Err(HarnessError::conflict("Native session has finished."));
        }
        self.input_used = self
            .input_used
            .checked_add(bytes.len())
            .ok_or_else(|| invalid("the input byte count overflows"))?;
        if self.input_limit.is_some_and(|n| self.input_used > n) {
            return Err(HarnessError::unavailable("Native input limit reached."));
        }
        let sender = self
            .input
            .as_ref()
            .ok_or_else(|| HarnessError::conflict("Native stdin is closed."))?;
        sender
            .try_send(bytes.to_vec())
            .map_err(|_| HarnessError::unavailable("Native stdin queue is unavailable or full."))
    }
    pub fn close_input(&mut self) {
        self.input.take();
    }
    fn poll_writer(&mut self) -> Result<()> {
        if self.writer_done {
            return Ok(());
        }
        match self.writer_result.try_recv() {
            Ok(result) => {
                self.writer_done = true;
                result
            }
            Err(mpsc::TryRecvError::Empty) => Ok(()),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.writer_done = true;
                Err(HarnessError::unavailable(
                    "Native stdin worker is unavailable.",
                ))
            }
        }
    }
    fn admit(&self) -> Result<()> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(HarnessError {
                status: 409,
                code: "native-cancelled",
                message: "Cancellation requested.".into(),
            });
        }
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
            || self
                .limits
                .seconds
                .is_some_and(|seconds| self.started.elapsed() >= Duration::from_secs(seconds))
        {
            return Err(timeout_error());
        }
        Ok(())
    }
    async fn read_chunk(&mut self) -> Result<Option<(Vec<u8>, bool)>> {
        loop {
            self.admit()?;
            if self.eof {
                return Ok(None);
            }
            self.poll_writer()?;
            let mut available = 0u32;
            if unsafe {
                PeekNamedPipe(
                    self.stdout.as_raw_handle(),
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    &mut available,
                    ptr::null_mut(),
                )
            } == 0
            {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) {
                    self.eof = true;
                    return Ok(None);
                }
                return Err(error.into());
            }
            if available == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
            let mut chunk = [0u8; 65536];
            let wanted = (available as usize).min(chunk.len());
            let count = self.stdout.read(&mut chunk[..wanted])?;
            if count == 0 {
                self.eof = true;
                return Ok(None);
            }
            let accepted = count.min(self.limits.output.saturating_sub(self.output.len()));
            let bytes = chunk[..accepted].to_vec();
            self.output.extend_from_slice(&bytes);
            return Ok(Some((bytes, accepted < count)));
        }
    }
    pub async fn next_line(&mut self) -> Result<Option<Vec<u8>>> {
        loop {
            self.admit()?;
            if let Some(end) = self.line.iter().position(|b| *b == b'\n') {
                if end > self.limits.line {
                    return Err(HarnessError::unavailable(
                        "Native output line limit reached.",
                    ));
                }
                return Ok(Some(self.line.drain(..=end).collect()));
            }
            if self.line.len() > self.limits.line {
                return Err(HarnessError::unavailable(
                    "Native output line limit reached.",
                ));
            }
            if self.eof {
                return Ok((!self.line.is_empty()).then(|| mem::take(&mut self.line)));
            }
            match self.read_chunk().await? {
                None => {}
                Some((_, true)) => return Err(output_error()),
                Some((bytes, false)) => self.line.extend(bytes),
            }
        }
    }
    pub async fn grace(&mut self, duration: Duration) -> Result<()> {
        self.close_input();
        let until = Instant::now() + duration;
        while Instant::now() < until && !self.eof {
            let remaining = until.saturating_duration_since(Instant::now());
            match tokio::time::timeout(remaining, self.next_line()).await {
                Ok(Ok(Some(_))) => {}
                Ok(Err(error)) => return Err(error),
                _ => break,
            }
        }
        Ok(())
    }
    pub async fn finish(&mut self) -> Result<Output> {
        if self.finished {
            return Err(HarnessError::conflict("Native session already finished."));
        }
        self.close_input();
        let cleanup = self.owner.cleanup().await;
        self.finished = true;
        let (kind, code) = cleanup?;
        let until = Instant::now() + Duration::from_secs(1);
        let mut writer_error = None;
        while !self.writer_done {
            if let Err(error) = self.poll_writer() {
                writer_error = Some(error);
                break;
            }
            if self.writer_done {
                break;
            }
            if Instant::now() >= until {
                STOPPING.store(true, Ordering::Release);
                return Err(unconfirmed());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if let Some(observer) = &self.observer {
            observer.boundary(
                "exited",
                &serde_json::json!({"code":code,"receipt_kind":kind}),
            )?;
            observer.boundary("cleanup_confirmed", &serde_json::json!({}))?;
        }
        if let Some(error) = writer_error {
            return Err(error);
        }
        if kind == 1 {
            return Err(HarnessError::unavailable(
                "Native guardian could not launch the command.",
            ));
        }
        Ok(Output {
            code,
            bytes: mem::take(&mut self.output),
            seconds: self.started.elapsed().as_secs_f64(),
        })
    }
}
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
    runner_lock: Option<Arc<File>>,
    event: impl FnMut(&[u8]) -> Result<()> + Send,
) -> Result<Output> {
    run_environment(
        argv,
        cwd,
        environment,
        &[],
        input,
        limits,
        cancel,
        runner_lock,
        event,
    )
    .await
}
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
    runner_lock: Option<Arc<File>>,
    event: impl FnMut(&[u8]) -> Result<()> + Send,
) -> Result<Output> {
    run_monitored(
        argv,
        cwd,
        environment,
        remove,
        input,
        limits,
        cancel,
        runner_lock,
        None,
        event,
    )
    .await
}
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
    runner_lock: Option<Arc<File>>,
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
    runner_lock: Option<Arc<File>>,
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
    runner_lock: Option<Arc<File>>,
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
    runner_lock: Option<Arc<File>>,
    observer: Option<Arc<dyn RunObserver>>,
    raw: bool,
    deadline: Option<Instant>,
    mut event: impl FnMut(&[u8]) -> Result<()> + Send,
) -> Result<Output> {
    let mut session = NativeSession::spawn_options_until(
        argv,
        cwd,
        environment,
        remove,
        limits,
        cancel,
        runner_lock,
        None,
        raw,
        observer,
        deadline,
    )?;
    let execute = async {
        session.write(input)?;
        session.close_input();
        if raw {
            while let Some((bytes, overflow)) = session.read_chunk().await? {
                if !bytes.is_empty() {
                    event(&bytes)?;
                }
                if overflow {
                    return Err(output_error());
                }
            }
        } else {
            while let Some(packet) = session.next_line().await? {
                event(&packet)?;
            }
        }
        Ok(())
    }
    .await;
    let finished = session.finish().await;
    match (finished, execute) {
        (Err(e), _) if e.code == "cleanup-unconfirmed" => Err(e),
        (_, Err(e)) => Err(e),
        (result, _) => result,
    }
}
fn packets(values: &[Value]) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for value in values {
        let packet = serde_json::to_vec(value)
            .map_err(|_| HarnessError::invalid("Invalid native JSON message."))?;
        if bytes.len().saturating_add(packet.len() + 1) > INPUT_LIMIT {
            return Err(HarnessError::unavailable("Native input limit reached."));
        }
        bytes.extend(packet);
        bytes.push(b'\n');
    }
    Ok(bytes)
}
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
    runner_lock: Option<Arc<File>>,
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
    runner_lock: Option<Arc<File>>,
    mut answer: impl FnMut(Value) -> Result<Control> + Send,
) -> Result<Value> {
    let mut session = NativeSession::spawn_options(
        argv,
        cwd,
        environment,
        remove,
        limits,
        cancel,
        runner_lock,
        Some(INPUT_LIMIT),
        false,
        None,
    )?;
    let execute = async {
        session.write(&packets(first)?)?;
        while let Some(line) = session.next_line().await? {
            let value = serde_json::from_slice(&line)
                .map_err(|_| HarnessError::invalid("Native control response is not valid JSON."))?;
            match answer(value)? {
                Control::Continue => {}
                Control::Write(values) => session.write(&packets(&values)?)?,
                Control::Done(value) => {
                    session.grace(CLEANUP).await?;
                    return Ok(value);
                }
            }
        }
        Err(HarnessError::unavailable(
            "Native control stream ended before the response.",
        ))
    }
    .await;
    let cleanup = session.finish().await;
    match (cleanup, execute) {
        (Err(e), _) if e.code == "cleanup-unconfirmed" => Err(e),
        (_, Err(e)) => Err(e),
        (Err(e), _) => Err(e),
        (Ok(_), Ok(value)) => Ok(value),
    }
}
unsafe extern "system" fn guardian_control(signal: u32) -> i32 {
    if signal == 0 || signal == 1 { 1 } else { 0 }
}

fn inherited(raw: usize) -> Result<Handle> {
    let raw = raw as HANDLE;
    if raw.is_null() || raw == INVALID_HANDLE_VALUE {
        return Err(invalid("an inherited handle is null"));
    }
    let mut flags = 0;
    if unsafe { GetHandleInformation(raw, &mut flags) } == 0 {
        return Err(invalid("an inherited handle is not open"));
    }
    Ok(unsafe { Handle::adopt(raw) })
}
fn receipt(file: &mut File, kind: u32, code: i32) -> Result<()> {
    let mut packet = Vec::with_capacity(12);
    packet.extend(RECEIPT_MAGIC);
    packet.extend(kind.to_le_bytes());
    packet.extend(code.to_le_bytes());
    file.write_all(&packet)?;
    file.flush()?;
    Ok(())
}
fn capture_process_code(process: &Handle) -> Result<Option<i32>> {
    if !exited(process)? {
        return Ok(None);
    }
    let mut code = 0u32;
    if unsafe { GetExitCodeProcess(process.raw(), &mut code) } == 0 {
        return Err(os_error());
    }
    Ok(Some(code as i32))
}
fn capture_receipt(file: &mut File, kind: u32, code: i32) -> Result<()> {
    let mut packet = Vec::with_capacity(12);
    packet.extend(CAPTURE_RECEIPT_MAGIC);
    packet.extend(kind.to_le_bytes());
    packet.extend(code.to_le_bytes());
    file.write_all(&packet)?;
    file.flush()?;
    Ok(())
}
fn close_capture_stdio() -> Result<()> {
    let mut handles = Vec::new();
    let mut failure = None;
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        let handle = unsafe { GetStdHandle(which) };
        if unsafe { SetStdHandle(which, ptr::null_mut()) } == 0 && failure.is_none() {
            failure = Some(os_error());
        }
        if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
            handles.push(handle);
        }
    }
    handles.sort_by_key(|handle| *handle as usize);
    handles.dedup();
    for handle in handles {
        if unsafe { CloseHandle(handle) } == 0 && failure.is_none() {
            failure = Some(os_error());
        }
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Internal trusted entrypoint. All target processes join the supplied Job at
/// creation time and remain suspended until membership is verified.
pub fn guard_entry(arguments: &[String]) -> Result<i32> {
    let mut raw = BTreeMap::<String, usize>::new();
    let mut index = 0;
    let mut interactive = false;
    while index < arguments.len() && arguments[index] != "--" {
        let key = &arguments[index];
        if key == "--interactive" {
            if interactive {
                return Err(invalid("--interactive repeats"));
            }
            interactive = true;
            index += 1;
            continue;
        }
        if ![
            "--watchdog-handle",
            "--completion-handle",
            "--job-handle",
            "--runner-lock-handle",
            "--native-exit-handle",
        ]
        .contains(&key.as_str())
            || index + 1 >= arguments.len()
            || raw.contains_key(key)
        {
            return Err(invalid(
                "a guardian option is unknown, repeated or has no value",
            ));
        }
        let value = arguments[index + 1]
            .parse::<usize>()
            .map_err(|_| invalid("a guardian handle is not a number"))?;
        raw.insert(key.clone(), value);
        index += 2;
    }
    if arguments.get(index).map(String::as_str) != Some("--") || index + 1 >= arguments.len() {
        return Err(invalid(
            "the guardian command has no -- separator or no program",
        ));
    }
    for required in ["--watchdog-handle", "--completion-handle", "--job-handle"] {
        if !raw.contains_key(required) {
            return Err(invalid("a required guardian handle is missing"));
        }
    }
    let mut values = raw.values().copied().collect::<Vec<_>>();
    values.sort_unstable();
    values.dedup();
    if values.len() != raw.len() {
        return Err(invalid("guardian handles repeat"));
    }
    let watch = inherited(raw["--watchdog-handle"])?;
    let status = inherited(raw["--completion-handle"])?;
    let job = inherited(raw["--job-handle"])?;
    let native_status = raw
        .get("--native-exit-handle")
        .copied()
        .map(inherited)
        .transpose()?;
    if native_status.is_some() && interactive {
        return Err(invalid("an interactive run cannot capture the native exit"));
    }
    let capture_lifetime = native_status.is_some();
    let lock = raw
        .get("--runner-lock-handle")
        .copied()
        .map(inherited)
        .transpose()?;
    if unsafe { windows_sys::Win32::Storage::FileSystem::GetFileType(watch.raw()) }
        != windows_sys::Win32::Storage::FileSystem::FILE_TYPE_PIPE
        || unsafe { windows_sys::Win32::Storage::FileSystem::GetFileType(status.raw()) }
            != windows_sys::Win32::Storage::FileSystem::FILE_TYPE_PIPE
    {
        return Err(invalid("a guardian watch or status handle is not a pipe"));
    }
    if native_status.as_ref().is_some_and(|handle| unsafe {
        windows_sys::Win32::Storage::FileSystem::GetFileType(handle.raw())
    } != windows_sys::Win32::Storage::FileSystem::FILE_TYPE_PIPE) {
        return Err(invalid("the native exit handle is not a pipe"));
    }
    for handle in [&watch, &status, &job] {
        inheritable(handle.raw(), false)?
    }
    if let Some(handle) = &native_status {
        inheritable(handle.raw(), false)?;
    }
    if let Some(lock) = &lock {
        inheritable(lock.raw(), false)?;
    }
    let mut completion = status.file();
    let dead = Arc::new(AtomicBool::new(false));
    let flag = dead.clone();
    let mut watcher = watch.file();
    let mut native_process: Option<Handle> = None;
    let mut result = (|| -> Result<i32> {
        if !job_empty(&job)? {
            return Err(HarnessError::invalid("Native custody Job is not empty."));
        }
        std::thread::Builder::new()
            .name("infra-owner-watchdog".into())
            .spawn(move || {
                let mut buffer = [0u8; 64];
                loop {
                    match watcher.read(&mut buffer) {
                        Ok(n) if n > 0 => {}
                        _ => break,
                    }
                }
                flag.store(true, Ordering::Release);
            })?;
        let argv = &arguments[index + 1..];
        if argv.iter().any(|s| s.contains('\0')) {
            return Err(invalid("a guarded argument contains NUL"));
        }
        if dead.load(Ordering::Acquire) {
            return Ok(1);
        }
        let cwd = std::env::current_dir()?;
        let (env, pairs) = environment(&BTreeMap::new(), &[])?;
        let executable = resolve(&argv[0], &cwd, &pairs)?;
        let stdin = Handle::duplicate(unsafe { GetStdHandle(STD_INPUT_HANDLE) }, true)?;
        let stdout = Handle::duplicate(unsafe { GetStdHandle(STD_OUTPUT_HANDLE) }, true)?;
        let stderr = Handle::duplicate(unsafe { GetStdHandle(STD_ERROR_HANDLE) }, true)?;
        let child_lock = lock
            .as_ref()
            .map(|h| Handle::duplicate(h.raw(), true))
            .transpose()?;
        let extras = child_lock
            .as_ref()
            .map(|h| vec![h.raw()])
            .unwrap_or_default();
        let arguments = argv[1..].iter().map(OsString::from).collect::<Vec<_>>();
        if interactive
            && unsafe {
                windows_sys::Win32::System::Console::SetConsoleCtrlHandler(
                    Some(guardian_control),
                    1,
                )
            } == 0
        {
            return Err(os_error());
        }
        let launched = create(
            &executable,
            &arguments,
            &cwd,
            &env,
            [stdin.raw(), stdout.raw(), stderr.raw()],
            &extras,
            Some(&job),
            !interactive,
        );
        drop(stdin);
        drop(stdout);
        drop(stderr);
        drop(child_lock);
        if capture_lifetime {
            close_capture_stdio()?;
        }
        let child = launched?;
        if capture_lifetime {
            native_process = Some(child.process);
            loop {
                if let Some(code) = capture_process_code(native_process.as_ref().unwrap())? {
                    return Ok(code);
                }
                if dead.load(Ordering::Acquire) {
                    return Ok(1);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        loop {
            if dead.load(Ordering::Acquire) {
                return Ok(1);
            }
            if exited(&child.process)? {
                let mut code = 0;
                if unsafe { GetExitCodeProcess(child.process.raw(), &mut code) } == 0 {
                    return Err(os_error());
                }
                return Ok(code as i32);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    let mut capture_completion = native_status.map(Handle::file);
    if capture_lifetime {
        if let Err(error) = close_capture_stdio() {
            result = Err(error);
        }
        if let Some(process) = &native_process {
            match capture_process_code(process) {
                Ok(Some(code)) => {
                    let mut observation = capture_completion.take().unwrap();
                    let written = capture_receipt(&mut observation, 0, code);
                    drop(observation);
                    if let Err(error) = written {
                        result = Err(error);
                    } else {
                        if result.is_ok() {
                            result = Ok(code);
                        }
                        // A natural native exit does not authorize ending the
                        // still-writing descendants' captured stream lifetime.
                        while !dead.load(Ordering::Acquire) {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                    }
                }
                Ok(None) => {}
                Err(error) => result = Err(error),
            }
        } else {
            // No native handle was ever created. Do not occupy this lane with
            // a synthetic stop code when an actual native status may follow.
            let kind = if result.is_err() { 1 } else { 2 };
            let mut observation = capture_completion.take().unwrap();
            if let Err(error) = capture_receipt(&mut observation, kind, 1) {
                result = Err(error);
            }
        }
    }
    kill_job(&job);
    loop {
        if job_empty(&job).unwrap_or(false) {
            break;
        }
        kill_job(&job);
        std::thread::sleep(Duration::from_millis(50));
    }
    if let Some(mut observation) = capture_completion {
        // The retained native HANDLE survives Job termination. Query its
        // signaled state and actual exit code, even if parent stop won the
        // observation race; never use the guardian's synthetic stop result.
        let observed = (|| -> Result<i32> {
            let process = native_process
                .as_ref()
                .ok_or_else(|| invalid("no native process handle was retained"))?;
            if unsafe { WaitForSingleObject(process.raw(), 1000) } != WAIT_OBJECT_0 {
                return Err(HarnessError::unavailable(
                    "Captured native process did not signal after Job cleanup.",
                ));
            }
            capture_process_code(process)?.ok_or_else(|| {
                HarnessError::unavailable("Captured native process has no exit status.")
            })
        })();
        let (kind, code) = match &observed {
            Ok(code) => (0, *code),
            Err(_) => (1, 1),
        };
        if let Err(error) = capture_receipt(&mut observation, kind, code) {
            result = Err(error);
        } else if let Err(error) = observed {
            result = Err(error);
        } else if result.is_ok() {
            result = Ok(code);
        }
    }
    let kind = if result.is_err() {
        1
    } else if dead.load(Ordering::Acquire) {
        2
    } else {
        0
    };
    let code = result.unwrap_or(1);
    receipt(&mut completion, kind, code)?;
    drop(lock);
    drop(job);
    Ok(0)
}
