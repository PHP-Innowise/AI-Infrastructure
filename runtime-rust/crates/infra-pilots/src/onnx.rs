use crate::{
    excerpt::{ExcerptScorer, ScoreFailure},
    strict,
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};
#[cfg(feature = "onnx")]
use std::{collections::BTreeSet, fs};
pub const MAX_PASSAGES: usize = 32;
/// Whether this build contains inference itself. An external scorer worker
/// may provide it independently, even when this dependency was built without it.
pub const fn compiled() -> bool {
    cfg!(feature = "onnx")
}
const REQUEST: usize = 48 * 1024;
const RESPONSE: usize = 16 * 1024;
static OWNER: Mutex<Option<PendingWorker>> = Mutex::new(None);
#[cfg(feature = "onnx")]
const JINA: &[(&str, &str, u64)] = &[
    (
        "tokenizer.json",
        "0046da43cc8c424b317f56b092b0512aaaa65c4f925d2f16af9d9eeb4d0ef902",
        2030772,
    ),
    (
        "config.json",
        "dc70646aa6c9e75e3c513cc9c037f35ad54308001c3961d45c0f69749bcfb022",
        1206,
    ),
    (
        "tokenizer_config.json",
        "d291c6652d96d56ffdbcf1ea19d9bae5ed79003f7648c627e725a619227ce8fa",
        1215,
    ),
    (
        "special_tokens_map.json",
        "06e405a36dfe4b9604f484f6a1e619af1a7f7d09e34a8555eb0b77b66318067f",
        280,
    ),
    (
        "model.onnx",
        "e0e743251c7566e2b1e4f5ad091c681a700d7d7a3d85541ea56ca3acf43d1afa",
        132350375,
    ),
];
pub fn valid_request(query: &str, passages: &[String]) -> bool {
    !query.is_empty()
        && query.chars().count() <= 1024
        && !passages.is_empty()
        && passages.len() <= MAX_PASSAGES
        && passages
            .iter()
            .all(|p| !p.is_empty() && p.chars().count() <= 1200)
}
fn failure(reason: &'static str) -> ScoreFailure {
    ScoreFailure { reason }
}
pub struct Scorer {
    pub assets: PathBuf,
    pub worker: PathBuf,
    pub ort_library: PathBuf,
    pub timeout: Duration,
}
struct PendingWorker {
    child: Child,
    reaped: bool,
    io_failed: bool,
    #[cfg(windows)]
    job: Option<WindowsJob>,
    #[cfg(any(not(unix), test))]
    threads: Vec<std::thread::JoinHandle<()>>,
}
impl PendingWorker {
    fn new(child: Child) -> Self {
        Self {
            child,
            reaped: false,
            io_failed: false,
            #[cfg(windows)]
            job: None,
            #[cfg(any(not(unix), test))]
            threads: Vec::new(),
        }
    }
}
fn finish_worker(
    slot: &mut Option<PendingWorker>,
    deadline: Instant,
) -> std::result::Result<bool, ScoreFailure> {
    let Some(worker) = slot.as_mut() else {
        return Ok(true);
    };
    drop(worker.child.stdin.take());
    drop(worker.child.stdout.take());
    drop(worker.child.stderr.take());
    #[cfg(windows)]
    if let Some(job) = worker.job.as_ref() {
        job.terminate();
    }
    if !worker.reaped {
        // WNOWAIT proves the leader is still our waitable child. Keep that
        // identity pinned until its process group has received cleanup.
        #[cfg(unix)]
        if observe_exit(&worker.child).is_err() {
            return Ok(false);
        }
        kill(&mut worker.child);
    }
    loop {
        // Poll once even after expiry. An unconfirmed child stays owned by the
        // single slot, and the next call spends its own deadline recovering it.
        if !worker.reaped {
            match worker.child.try_wait() {
                Ok(Some(_)) => worker.reaped = true,
                Ok(None) => {}
                Err(_) => return Ok(false),
            }
        }
        #[cfg(any(not(unix), test))]
        {
            let mut index = 0;
            while index < worker.threads.len() {
                if worker.threads[index].is_finished() {
                    if worker.threads.swap_remove(index).join().is_err() {
                        worker.io_failed = true;
                    }
                } else {
                    index += 1;
                }
            }
        }
        #[cfg(all(unix, not(test)))]
        let io_finished = true;
        #[cfg(any(not(unix), test))]
        let io_finished = worker.threads.is_empty();
        if worker.reaped && io_finished {
            let io_failed = worker.io_failed;
            *slot = None;
            return if io_failed {
                Err(failure("worker-exit"))
            } else {
                Ok(true)
            };
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(1).min(left));
    }
}
impl ExcerptScorer for Scorer {
    fn score(
        &mut self,
        query: &str,
        passages: &[String],
        remaining: Duration,
    ) -> std::result::Result<Vec<f64>, ScoreFailure> {
        if !valid_request(query, passages) {
            return Err(failure("invalid-request"));
        }
        let timeout = self.timeout.min(Duration::from_secs(5)).min(remaining);
        if timeout.is_zero() {
            return Err(failure("timeout"));
        }
        let payload = serde_json::to_vec(&json!({"query":query,"passages":passages}))
            .map_err(|_| failure("invalid-request"))?;
        if payload.len() > REQUEST {
            return Err(failure("request-too-large"));
        }
        if !self.worker.is_file() {
            return Err(failure("worker-unavailable"));
        }
        let deadline = Instant::now() + timeout;
        let reserve = Duration::from_millis(150).min(timeout / 5);
        let work_deadline = deadline - reserve;
        let mut owned = match OWNER.try_lock() {
            Ok(owned) => owned,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(failure("busy")),
        };
        if !finish_worker(&mut owned, deadline)? {
            return Err(failure("worker-unreaped"));
        }
        if Instant::now() >= work_deadline {
            return Err(failure("timeout"));
        }
        let mut cmd = Command::new(&self.worker);
        cmd.args(["onnx-worker", "--assets"])
            .arg(&self.assets)
            .arg("--ort-library")
            .arg(&self.ort_library)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("OMP_NUM_THREADS", "1")
            .env("TOKENIZERS_PARALLELISM", "false")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
            #[cfg(target_os = "linux")]
            {
                let parent = std::process::id() as libc::pid_t;
                unsafe {
                    cmd.pre_exec(move || {
                        if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0
                            || libc::getppid() != parent
                        {
                            return Err(std::io::Error::other("worker owner changed"));
                        }
                        Ok(())
                    });
                }
            }
        }
        if Instant::now() >= work_deadline {
            return Err(failure("timeout"));
        }
        let child = cmd.spawn().map_err(|_| failure("worker-unavailable"))?;
        *owned = Some(PendingWorker::new(child));
        let output = (|| {
            let worker = owned
                .as_mut()
                .ok_or_else(|| failure("worker-unavailable"))?;
            #[cfg(windows)]
            {
                worker.job = Some(
                    WindowsJob::attach(&worker.child).map_err(|_| failure("worker-unavailable"))?,
                );
            }
            pipes(worker, &payload, work_deadline)
        })();
        if !finish_worker(&mut owned, deadline)? {
            return Err(failure("worker-unreaped"));
        }
        let raw = output?;
        let response =
            strict::decode(&raw, RESPONSE, 16, 4096).map_err(|_| failure("invalid-output"))?;
        if let Some(reason) = response.get("error").and_then(Value::as_str) {
            let reason = match reason {
                "asset-missing" => "asset-missing",
                "dependency-unavailable" => "dependency-unavailable",
                "hash-mismatch" => "hash-mismatch",
                "invalid-assets" => "invalid-assets",
                "invalid-request" => "invalid-request",
                "invalid-shape" => "invalid-shape",
                "inference-failed" => "inference-failed",
                _ => return Err(failure("invalid-output")),
            };
            if response.as_object().is_none_or(|m| m.len() != 1) {
                return Err(failure("invalid-output"));
            }
            return Err(failure(reason));
        }
        strict::keys(&response, &["scores"], &["scores"]).map_err(|_| failure("invalid-output"))?;
        let scores = response["scores"]
            .as_array()
            .ok_or_else(|| failure("invalid-output"))?;
        if scores.len() != passages.len() {
            return Err(failure("invalid-output"));
        }
        scores
            .iter()
            .map(|v| {
                v.as_f64()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| failure("invalid-output"))
            })
            .collect()
    }
}
fn kill(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
    }
    let _ = child.kill();
}
#[cfg(unix)]
fn observe_exit(child: &Child) -> std::result::Result<Option<bool>, ScoreFailure> {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            child.id() as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result != 0 {
        return Err(failure("worker-exit"));
    }
    let pid = unsafe { info.si_pid() };
    if pid == 0 {
        return Ok(None);
    }
    if pid != child.id() as libc::pid_t {
        return Err(failure("worker-exit"));
    }
    match info.si_code {
        libc::CLD_EXITED => Ok(Some(unsafe { info.si_status() } == 0)),
        libc::CLD_KILLED | libc::CLD_DUMPED => Ok(Some(false)),
        _ => Err(failure("worker-exit")),
    }
}
#[cfg(unix)]
fn pipes(
    worker: &mut PendingWorker,
    payload: &[u8],
    deadline: Instant,
) -> std::result::Result<Vec<u8>, ScoreFailure> {
    use std::os::fd::AsRawFd;
    let child = &mut worker.child;
    let input = child
        .stdin
        .take()
        .ok_or_else(|| failure("worker-unavailable"))?;
    let mut output = child
        .stdout
        .take()
        .ok_or_else(|| failure("worker-unavailable"))?;
    for fd in [input.as_raw_fd(), output.as_raw_fd()] {
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
                return Err(failure("worker-unavailable"));
            }
        }
    }
    let mut sent = 0;
    let mut open = Some(input);
    let mut eof = false;
    let mut result = vec![];
    loop {
        if Instant::now() >= deadline {
            return Err(failure("timeout"));
        }
        let mut progress = false;
        if let Some(stdin) = open.as_mut() {
            match stdin.write(&payload[sent..(sent + 4096).min(payload.len())]) {
                Ok(0) => return Err(failure("worker-exit")),
                Ok(n) => {
                    sent += n;
                    progress = true;
                    if sent == payload.len() {
                        open.take();
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => (),
                Err(_) => return Err(failure("worker-exit")),
            }
        }
        let mut b = [0u8; 4096];
        if !eof {
            match output.read(&mut b) {
                Ok(0) => eof = true,
                Ok(n) => {
                    result.extend_from_slice(&b[..n]);
                    progress = true;
                    if result.len() > RESPONSE {
                        return Err(failure("output-too-large"));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => (),
                Err(_) => return Err(failure("worker-exit")),
            }
        }
        if eof && let Some(success) = observe_exit(child)? {
            if !success || open.is_some() {
                return Err(failure("worker-exit"));
            }
            return Ok(result);
        }
        if !progress {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
#[cfg(not(unix))]
fn pipes(
    worker: &mut PendingWorker,
    payload: &[u8],
    deadline: Instant,
) -> std::result::Result<Vec<u8>, ScoreFailure> {
    let child = &mut worker.child;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| failure("worker-unavailable"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| failure("worker-unavailable"))?;
    let payload = payload.to_vec();
    enum Message {
        Written(std::io::Result<()>),
        Read(std::io::Result<()>, Vec<u8>),
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let writer_tx = tx.clone();
    let writer = std::thread::Builder::new()
        .name("onnx-input".into())
        .spawn(move || {
            let result = stdin.write_all(&payload);
            drop(stdin);
            let _ = writer_tx.send(Message::Written(result));
        })
        .map_err(|_| failure("worker-unavailable"))?;
    worker.threads.push(writer);
    let reader = std::thread::Builder::new()
        .name("onnx-output".into())
        .spawn(move || {
            let mut b = vec![];
            let result = stdout
                .by_ref()
                .take((RESPONSE + 1) as u64)
                .read_to_end(&mut b)
                .map(|_| ());
            let _ = tx.send(Message::Read(result, b));
        })
        .map_err(|_| failure("worker-unavailable"))?;
    worker.threads.push(reader);
    let mut written = false;
    let mut output = None;
    while !written || output.is_none() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(failure("timeout"));
        }
        match rx.recv_timeout(left) {
            Ok(Message::Written(result)) => {
                result.map_err(|_| failure("worker-exit"))?;
                written = true;
            }
            Ok(Message::Read(result, b)) => {
                result.map_err(|_| failure("worker-exit"))?;
                if b.len() > RESPONSE {
                    return Err(failure("output-too-large"));
                }
                output = Some(b);
            }
            Err(_) => return Err(failure("timeout")),
        }
    }
    // EOF and successful delivery alone do not authorize scores. Observe the
    // natural exit before Job cleanup can replace it with a termination code.
    loop {
        if Instant::now() >= deadline {
            return Err(failure("timeout"));
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                worker.reaped = true;
                if !status.success() {
                    return Err(failure("worker-exit"));
                }
                break;
            }
            Ok(None) => {
                let left = deadline.saturating_duration_since(Instant::now());
                std::thread::sleep(Duration::from_millis(1).min(left));
            }
            Err(_) => return Err(failure("worker-exit")),
        }
    }
    Ok(output.unwrap())
}
#[cfg(windows)]
struct WindowsJob(std::os::windows::io::OwnedHandle);
#[cfg(windows)]
impl WindowsJob {
    fn attach(child: &Child) -> std::io::Result<Self> {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let owned = Self(OwnedHandle::from_raw_handle(job.cast()));
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(job, child.as_raw_handle() as _) == 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(owned)
        }
    }
    fn terminate(&self) {
        use std::os::windows::io::AsRawHandle;
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(
                self.0.as_raw_handle().cast(),
                137,
            );
        }
    }
}
#[cfg(windows)]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(feature = "onnx")]
fn assets(dir: &Path) -> std::result::Result<(BTreeMap<String, PathBuf>, bool), &'static str> {
    let manifest = fs::read(dir.join("manifest.json")).map_err(|_| "asset-missing")?;
    let v = strict::decode(&manifest, 128 * 1024, 16, 4096).map_err(|_| "asset-missing")?;
    let jina = v.get("profile").is_some() || v.get("schema_version").is_some();
    if jina
        && (v["schema_version"] != 1
            || v["profile"] != "jina-tiny-fp32"
            || v["model_id"] != "jinaai/jina-reranker-v1-tiny-en"
            || v["revision"] != "aca45de6945b5dc6399abcd2a9c55ded5dc9111f"
            || v["variant"] != "onnx/model.onnx"
            || v["pad_token"] != "<pad>"
            || v["pad_id"] != 1)
    {
        return Err("invalid-assets");
    }
    let required = if jina {
        JINA.iter().map(|x| x.0).collect::<BTreeSet<_>>()
    } else {
        [
            "model_O3.onnx",
            "tokenizer.json",
            "config.json",
            "tokenizer_config.json",
            "special_tokens_map.json",
        ]
        .into_iter()
        .collect()
    };
    let files = v["files"].as_object().ok_or("invalid-assets")?;
    if files.keys().map(String::as_str).collect::<BTreeSet<_>>() != required {
        return Err("invalid-assets");
    }
    let base = infra_core::fs::canonicalize(dir).map_err(|_| "asset-missing")?;
    let mut out = BTreeMap::new();
    for (name, meta) in files {
        if Path::new(name).file_name().and_then(|n| n.to_str()) != Some(name) {
            return Err("invalid-assets");
        }
        let digest = meta["sha256"]
            .as_str()
            .filter(|s| strict::digest(s))
            .ok_or("invalid-assets")?;
        let size = meta["size"]
            .as_u64()
            .filter(|s| *s > 0)
            .ok_or("invalid-assets")?;
        if jina
            && !JINA
                .iter()
                .any(|(n, h, b)| *n == name && *h == digest && *b == size)
        {
            return Err("invalid-assets");
        }
        let p = infra_core::fs::canonicalize(dir.join(name)).map_err(|_| "asset-missing")?;
        if !p.starts_with(&base) || !p.is_file() {
            return Err("asset-missing");
        }
        if fs::metadata(&p).map_err(|_| "asset-missing")?.len() != size {
            return Err("hash-mismatch");
        }
        use sha2::{Digest, Sha256};
        let mut d = Sha256::new();
        let mut f = fs::File::open(&p).map_err(|_| "asset-missing")?;
        let mut b = [0u8; 65536];
        loop {
            let n = f.read(&mut b).map_err(|_| "asset-missing")?;
            if n == 0 {
                break;
            }
            d.update(&b[..n]);
        }
        if infra_core::domain::hex_lower(d.finalize()) != digest {
            return Err("hash-mismatch");
        }
        out.insert(name.clone(), p);
    }
    if jina {
        let read = |name: &str| -> std::result::Result<Value, &'static str> {
            strict::decode(
                &fs::read(&out[name]).map_err(|_| "invalid-assets")?,
                65536,
                16,
                4096,
            )
            .map_err(|_| "invalid-assets")
        };
        let config = read("config.json")?;
        let tokenizer = read("tokenizer_config.json")?;
        let special = read("special_tokens_map.json")?;
        let token = |v: &Value| {
            v.as_str()
                .map(str::to_owned)
                .or_else(|| v["content"].as_str().map(str::to_owned))
        };
        if config["pad_token_id"] != 0
            || tokenizer["tokenizer_class"] != "RobertaTokenizer"
            || token(&tokenizer["pad_token"]).as_deref() != Some("<pad>")
            || token(&special["pad_token"]).as_deref() != Some("<pad>")
        {
            return Err("invalid-assets");
        }
    }
    Ok((out, jina))
}
#[cfg(feature = "onnx")]
fn infer(
    dir: &Path,
    library: &Path,
    request: &Value,
) -> std::result::Result<Vec<f64>, &'static str> {
    use ort::{
        session::Session,
        value::{Tensor, TensorElementType, ValueType},
    };
    use tokenizers::{
        PaddingDirection, PaddingParams, PaddingStrategy, Tokenizer, TruncationDirection,
        TruncationParams, TruncationStrategy,
    };
    let (files, jina) = assets(dir)?;
    if !library.is_file() {
        return Err("dependency-unavailable");
    }
    let runtime =
        unsafe { libloading::Library::new(library) }.map_err(|_| "dependency-unavailable")?;
    let base = unsafe {
        runtime.get::<unsafe extern "system" fn() -> *const ort::sys::OrtApiBase>(b"OrtGetApiBase")
    }
    .map_err(|_| "dependency-unavailable")?;
    let api = unsafe { base() };
    if api.is_null() {
        return Err("dependency-unavailable");
    }
    let version = unsafe { std::ffi::CStr::from_ptr(((*api).GetVersionString)()) }
        .to_str()
        .map_err(|_| "dependency-unavailable")?;
    if jina && version != "1.31.0" {
        return Err("dependency-unavailable");
    }
    ort::init_from(library)
        .map_err(|_| "dependency-unavailable")?
        .with_execution_providers([ort::ep::CPU::default().build()])
        .with_telemetry(false)
        .commit();
    let mut session = Session::builder()
        .map_err(|_| "inference-failed")?
        .with_intra_threads(1)
        .map_err(|_| "inference-failed")?
        .with_inter_threads(1)
        .map_err(|_| "inference-failed")?
        .with_parallel_execution(false)
        .map_err(|_| "inference-failed")?
        .with_execution_providers([ort::ep::CPU::default().build()])
        .map_err(|_| "inference-failed")?
        .commit_from_file(&files[if jina { "model.onnx" } else { "model_O3.onnx" }])
        .map_err(|_| "inference-failed")?;
    let expected = if jina {
        ["input_ids", "attention_mask"]
            .into_iter()
            .collect::<BTreeSet<_>>()
    } else {
        ["input_ids", "attention_mask", "token_type_ids"]
            .into_iter()
            .collect()
    };
    if session
        .inputs()
        .iter()
        .map(|x| x.name())
        .collect::<BTreeSet<_>>()
        != expected
        || session.outputs().len() != 1
        || session.outputs()[0].name() != "logits"
    {
        return Err("invalid-shape");
    }
    if jina {
        for x in session.inputs() {
            match x.dtype() {
                ValueType::Tensor { ty, shape, .. }
                    if *ty == TensorElementType::Int64
                        && shape.len() == 2
                        && shape.iter().all(|d| *d < 0) => {}
                _ => return Err("invalid-shape"),
            }
        }
        match session.outputs()[0].dtype() {
            ValueType::Tensor { ty, shape, .. }
                if *ty == TensorElementType::Float32
                    && shape.len() == 2
                    && shape[0] < 0
                    && shape[1] == 1 => {}
            _ => return Err("invalid-shape"),
        }
    }
    let mut tokenizer =
        Tokenizer::from_file(&files["tokenizer.json"]).map_err(|_| "invalid-assets")?;
    if jina && tokenizer.token_to_id("<pad>") != Some(1) {
        return Err("invalid-assets");
    }
    tokenizer
        .with_truncation(Some(TruncationParams {
            max_length: 384,
            strategy: TruncationStrategy::LongestFirst,
            direction: TruncationDirection::Right,
            stride: 0,
        }))
        .map_err(|_| "invalid-assets")?;
    tokenizer.with_padding(Some(PaddingParams {
        strategy: PaddingStrategy::BatchLongest,
        direction: PaddingDirection::Right,
        pad_to_multiple_of: None,
        pad_id: if jina { 1 } else { 0 },
        pad_type_id: 0,
        pad_token: if jina { "<pad>" } else { "[PAD]" }.into(),
    }));
    let query = request["query"].as_str().ok_or("invalid-request")?;
    let passages = request["passages"].as_array().ok_or("invalid-request")?;
    let encodings = tokenizer
        .encode_batch(
            passages
                .iter()
                .map(|p| (query, p.as_str().unwrap()))
                .collect::<Vec<_>>(),
            true,
        )
        .map_err(|_| "inference-failed")?;
    let rows = encodings.len();
    let columns = encodings.first().map(|e| e.len()).ok_or("invalid-shape")?;
    if rows != passages.len()
        || columns == 0
        || columns > 384
        || encodings.iter().any(|e| e.len() != columns)
    {
        return Err("invalid-shape");
    }
    let tensor = |which: u8| -> std::result::Result<Tensor<i64>, &'static str> {
        let v = encodings
            .iter()
            .flat_map(|e| {
                match which {
                    0 => e.get_ids(),
                    1 => e.get_attention_mask(),
                    _ => e.get_type_ids(),
                }
                .iter()
                .map(|v| *v as i64)
            })
            .collect::<Vec<_>>();
        Tensor::from_array(([rows, columns], v)).map_err(|_| "inference-failed")
    };
    let ids = tensor(0)?;
    let mask = tensor(1)?;
    let outputs = if jina {
        session.run(ort::inputs! {"input_ids"=>ids,"attention_mask"=>mask})
    } else {
        session.run(
            ort::inputs! {"input_ids"=>ids,"attention_mask"=>mask,"token_type_ids"=>tensor(2)?},
        )
    }
    .map_err(|_| "inference-failed")?;
    let logits = &outputs["logits"];
    macro_rules! extract {
        ($ty:ty) => {
            if let Ok((shape, values)) = logits.try_extract_tensor::<$ty>() {
                if shape.as_ref() != [rows as i64, 1] || values.len() != rows {
                    return Err("invalid-shape");
                }
                let scores = values.iter().map(|v| *v as f64).collect::<Vec<_>>();
                if scores.iter().any(|v| !v.is_finite()) {
                    return Err("invalid-shape");
                }
                return Ok(scores);
            }
        };
    }
    extract!(f32);
    if !jina {
        macro_rules! extract_half {
            ($ty:ty) => {
                if let Ok((shape, values)) = logits.try_extract_tensor::<$ty>() {
                    if shape.as_ref() != [rows as i64, 1] || values.len() != rows {
                        return Err("invalid-shape");
                    }
                    let scores = values.iter().map(|v| v.to_f64()).collect::<Vec<_>>();
                    if scores.iter().any(|v| !v.is_finite()) {
                        return Err("invalid-shape");
                    }
                    return Ok(scores);
                }
            };
        }
        extract_half!(half::f16);
        extract_half!(half::bf16);
        extract!(f64);
        extract!(i8);
        extract!(i16);
        extract!(i32);
        extract!(i64);
        extract!(u8);
        extract!(u16);
        extract!(u32);
        extract!(u64);
    }
    Err("invalid-shape")
}
#[cfg(not(feature = "onnx"))]
fn infer(_: &Path, _: &Path, _: &Value) -> std::result::Result<Vec<f64>, &'static str> {
    Err("dependency-unavailable")
}
pub fn worker(args: &[String]) -> Result<i32> {
    let mut options = BTreeMap::new();
    let mut i = 0;
    while i < args.len() {
        let key = &args[i];
        if !["--assets", "--ort-library"].contains(&key.as_str()) {
            bail!("invalid worker argument");
        }
        i += 1;
        options.insert(
            key.clone(),
            PathBuf::from(args.get(i).context("worker option value")?),
        );
        i += 1;
    }
    let mut raw = vec![];
    std::io::stdin()
        .take((REQUEST + 1) as u64)
        .read_to_end(&mut raw)?;
    let response = (|| -> std::result::Result<Value, &'static str> {
        let request = strict::decode(&raw, REQUEST, 8, 256).map_err(|_| "invalid-request")?;
        strict::keys(&request, &["query", "passages"], &["query", "passages"])
            .map_err(|_| "invalid-request")?;
        let query = request["query"].as_str().ok_or("invalid-request")?;
        let a = request["passages"].as_array().ok_or("invalid-request")?;
        let passages = a
            .iter()
            .map(|v| v.as_str().map(str::to_owned).ok_or("invalid-request"))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if !valid_request(query, &passages) {
            return Err("invalid-request");
        }
        let assets = options.get("--assets").ok_or("asset-missing")?;
        let library = options
            .get("--ort-library")
            .ok_or("dependency-unavailable")?;
        Ok(json!({"scores":infer(assets,library,&request)?}))
    })();
    let body = match response {
        Ok(v) => v,
        Err(e) => json!({"error":e}),
    };
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer(&mut stdout, &body)?;
    stdout.write_all(b"\n")?;
    Ok(0)
}

#[cfg(all(test, target_os = "linux"))]
mod cleanup_tests {
    use super::{ExcerptScorer, PendingWorker, Scorer, finish_worker};
    use std::{
        collections::BTreeMap,
        fs::{self, File},
        io::{self, Read},
        os::{
            fd::{AsRawFd, FromRawFd, OwnedFd},
            unix::process::CommandExt,
        },
        path::{Path, PathBuf},
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    const CHILD: &str = "INFRA_ONNX_CUSTODY_TEST_CHILD";
    const TEST: &str = "onnx::cleanup_tests::tiny_timeouts_bound_pending_workers_and_later_recover";
    const MARKER: &str = "ONNX_PENDING_CUSTODY_OBSERVED";
    const DESC_TEST: &str =
        "onnx::cleanup_tests::exited_leader_descendants_are_retired_before_custody_release";
    const DESC_WORKER: &str = "onnx::cleanup_tests::descendant_worker_fixture";
    const DESC_MARKER: &str = "ONNX_DESCENDANT_CUSTODY_OBSERVED";
    #[derive(Clone, Debug)]
    struct Image {
        pid: u32,
        parent: u32,
        group: u32,
        birth: u64,
        state: String,
    }
    fn image(pid: u32) -> io::Result<Option<Image>> {
        let text = match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let invalid = || io::Error::other("Cannot parse owned ONNX test process identity");
        let end = text.rfind(')').ok_or_else(invalid)?;
        let fields = text[end + 1..].split_whitespace().collect::<Vec<_>>();
        Ok(Some(Image {
            pid,
            state: fields.first().ok_or_else(invalid)?.to_string(),
            parent: fields
                .get(1)
                .ok_or_else(invalid)?
                .parse()
                .map_err(|_| invalid())?,
            group: fields
                .get(2)
                .ok_or_else(invalid)?
                .parse()
                .map_err(|_| invalid())?,
            birth: fields
                .get(19)
                .ok_or_else(invalid)?
                .parse()
                .map_err(|_| invalid())?,
        }))
    }
    fn children() -> io::Result<BTreeMap<u32, Image>> {
        let mut found = BTreeMap::new();
        for task in fs::read_dir("/proc/self/task")? {
            let task = task?;
            let text = match fs::read_to_string(task.path().join("children")) {
                Ok(text) => text,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            for pid in text.split_whitespace() {
                let pid = pid
                    .parse::<u32>()
                    .map_err(|_| io::Error::other("Invalid owned child PID"))?;
                if let Some(current) = image(pid)? {
                    if current.parent != std::process::id() {
                        return Err(io::Error::other(
                            "ONNX test child belongs to another parent",
                        ));
                    }
                    found.insert(pid, current);
                }
            }
        }
        Ok(found)
    }
    fn reap_owned(wanted: &Image, until: Instant) -> io::Result<()> {
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, wanted.pid as libc::pid_t, 0) };
        if raw < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) && image(wanted.pid)?.is_none() {
                return Ok(());
            }
            return Err(error);
        }
        let pin = unsafe { OwnedFd::from_raw_fd(raw as i32) };
        let Some(current) = image(wanted.pid)? else {
            return Ok(());
        };
        if current.parent != std::process::id() || current.birth != wanted.birth {
            return Err(io::Error::other(
                "ONNX test child identity changed before cleanup",
            ));
        }
        let signalled = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                pin.as_raw_fd(),
                libc::SIGKILL,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if signalled < 0 && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
            return Err(io::Error::last_os_error());
        }
        loop {
            let mut status = 0;
            // Only a direct child can be reaped; the pidfd above pins its birth
            // and no test path signals a numeric PID or a process group.
            let reaped =
                unsafe { libc::waitpid(wanted.pid as libc::pid_t, &mut status, libc::WNOHANG) };
            if reaped == wanted.pid as libc::pid_t {
                return Ok(());
            }
            if reaped < 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ECHILD) && image(wanted.pid)?.is_none() {
                    return Ok(());
                }
                return Err(error);
            }
            if Instant::now() >= until {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Owned ONNX child cleanup timed out",
                ));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn child_body(root: &Path) {
        let before = children().unwrap();
        assert!(
            before.is_empty(),
            "isolated scorer child unexpectedly already owns processes"
        );
        let worker = ["/usr/bin/true", "/bin/true"]
            .into_iter()
            .map(PathBuf::from)
            .find(|path| path.is_file())
            .expect("native true fixture must exist");
        let mut scorer = Scorer {
            assets: root.join("unused-assets"),
            worker,
            ort_library: root.join("unused-library"),
            timeout: Duration::from_nanos(1),
        };
        let mut peak = 0;
        let mut unexpected_successes = 0;
        for _ in 0..100 {
            if scorer
                .score(
                    "owned cleanup fixture",
                    &["harmless passage".into()],
                    Duration::from_nanos(1),
                )
                .is_ok()
            {
                unexpected_successes += 1;
            }
            peak = peak.max(children().unwrap().len());
        }
        // Recovery gets this new call's ordinary budget; it must not create a
        // second worker while a previous worker remains unconfirmed.
        scorer.timeout = Duration::from_millis(250);
        let recovery = scorer.score(
            "owned cleanup fixture",
            &["harmless passage".into()],
            Duration::from_millis(250),
        );
        let after = children().unwrap();
        peak = peak.max(after.len());
        // Tiny calls may now refuse before spawn. Independently place a real,
        // exited but unreaped child in the cleanup slot, then pass a deadline
        // already in the past: the mandatory first poll must still reap it.
        let mut helper_errors = Vec::new();
        let mut helper = match Command::new(&scorer.worker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
        {
            Ok(child) => Some(PendingWorker::new(child)),
            Err(error) => {
                helper_errors.push(format!("helper spawn: {error}"));
                None
            }
        };
        let mut observed_zombie = false;
        if let Some(worker) = helper.as_ref() {
            let until = Instant::now() + Duration::from_secs(2);
            loop {
                match image(worker.child.id()) {
                    Ok(Some(current)) if current.state == "Z" => {
                        observed_zombie = true;
                        break;
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        helper_errors.push("helper disappeared without being reaped".into());
                        break;
                    }
                    Err(error) => {
                        helper_errors.push(format!("helper identity: {error}"));
                        break;
                    }
                }
                if Instant::now() >= until {
                    helper_errors.push("helper did not exit within two seconds".into());
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        let expired_poll_reaped =
            finish_worker(&mut helper, Instant::now() - Duration::from_secs(1)).unwrap_or(false);
        let retained_after_expired = helper.is_some();
        let helper_recovered =
            finish_worker(&mut helper, Instant::now() + Duration::from_secs(2)).unwrap_or(false);
        // Gather the failure evidence first, then clean every actual owned
        // child before any custody assertion can fail and unwind this test.
        let cleanup_until = Instant::now() + Duration::from_secs(2);
        let mut cleanup_errors = Vec::new();
        for process in children().unwrap().values() {
            if let Err(error) = reap_owned(process, cleanup_until) {
                cleanup_errors.push(format!("{}: {error}", process.pid));
            }
        }
        drop(helper);
        let remaining = children().unwrap();
        println!(
            "{MARKER} {}",
            serde_json::json!({
                "peak":peak,"after_recovery":after.len(),"unexpected_successes":unexpected_successes,
                "owned_zombies":after.values().filter(|process|process.state=="Z").count(),
                "cleanup_errors":cleanup_errors,"remaining_after_test_cleanup":remaining.len(),
                "recovery_error":recovery.err().map(|error|error.reason),
                "helper_observed_zombie":observed_zombie,"helper_expired_poll_reaped":expired_poll_reaped,
                "helper_retained_after_expired":retained_after_expired,"helper_recovered":helper_recovered,
                "helper_errors":helper_errors,
            })
        );
        assert!(
            cleanup_errors.is_empty() && remaining.is_empty(),
            "test cleanup did not reap its own children"
        );
        assert!(
            helper_errors.is_empty()
                && observed_zombie
                && expired_poll_reaped
                && !retained_after_expired
                && helper_recovered,
            "cleanup must poll a real exited child even with an already-past deadline"
        );
        assert_eq!(
            unexpected_successes, 0,
            "native true is not a scoring worker"
        );
        assert!(
            peak <= 1,
            "tiny scorer calls accumulated {peak} owned workers"
        );
        assert!(
            after.is_empty(),
            "ordinary recovery left {} owned workers",
            after.len()
        );
    }
    fn isolated(test: &str, marker: &str) {
        let root = tempfile::Builder::new()
            .prefix("infra-onnx-custody-")
            .tempdir()
            .unwrap();
        let out_path = root.path().join("child.stdout");
        let err_path = root.path().join("child.stderr");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture", "--test-threads=1"])
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env(CHILD, root.path())
            .env("HOME", root.path())
            .env("TMPDIR", root.path())
            .env("CODEX_HOME", root.path().join("codex"))
            .env("CLAUDE_CONFIG_DIR", root.path().join("claude"))
            .stdin(Stdio::null())
            .stdout(File::create(&out_path).unwrap())
            .stderr(File::create(&err_path).unwrap())
            .process_group(0)
            .spawn()
            .unwrap();
        let until = Instant::now() + Duration::from_secs(8);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= until {
                let cleanup = image(child.id()).and_then(|current| {
                    current.map_or(Ok(()), |current| {
                        reap_owned(&current, Instant::now() + Duration::from_secs(2))
                    })
                });
                panic!(
                    "isolated ONNX custody child exceeded eight seconds; targeted cleanup: {cleanup:?}"
                );
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        fn read(path: &Path) -> String {
            let mut bytes = Vec::new();
            File::open(path)
                .unwrap()
                .take(65537)
                .read_to_end(&mut bytes)
                .unwrap();
            assert!(bytes.len() <= 65536);
            String::from_utf8(bytes).unwrap()
        }
        let stdout = read(&out_path);
        let stderr = read(&err_path);
        print!("{stdout}");
        if !stderr.is_empty() {
            eprint!("{stderr}");
        }
        assert!(
            stdout.contains("running 1 test") && stdout.contains(marker),
            "exact child test did not execute: stdout={stdout}\nstderr={stderr}"
        );
        assert!(
            status.success() && stdout.contains("1 passed") && stdout.contains("0 failed"),
            "ONNX worker custody failed: stdout={stdout}\nstderr={stderr}"
        );
    }
    #[test]
    fn tiny_timeouts_bound_pending_workers_and_later_recover() {
        if let Some(root) = std::env::var_os(CHILD) {
            child_body(Path::new(&root));
        } else {
            isolated(TEST, MARKER);
        }
    }

    #[test]
    fn descendant_worker_fixture() {
        let Some(marker) = std::env::var_os("INFRA_ONNX_DESCENDANT_MARKER") else {
            return;
        };
        let hold_output = std::env::var_os("INFRA_ONNX_DESCENDANT_HOLD").as_deref()
            == Some(std::ffi::OsStr::new("1"));
        let mut request = Vec::new();
        std::io::stdin()
            .take(super::REQUEST as u64)
            .read_to_end(&mut request)
            .unwrap();
        let mut descriptors = [0; 2];
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        let mut reset: libc::sigaction = unsafe { std::mem::zeroed() };
        reset.sa_sigaction = libc::SIG_DFL;
        assert_eq!(unsafe { libc::sigemptyset(&mut reset.sa_mask) }, 0);
        let ready = [1u8];
        let ready_pointer = ready.as_ptr().cast::<libc::c_void>();
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0);
        if pid == 0 {
            // This is a multithreaded libtest process. Until _exit, the child
            // executes only async-signal-safe libc calls using prepared data.
            unsafe {
                libc::close(descriptors[0]);
                libc::close(0);
                if !hold_output {
                    libc::close(1);
                    libc::close(2);
                }
                if libc::sigaction(libc::SIGALRM, &reset, std::ptr::null_mut()) != 0 {
                    libc::_exit(126);
                }
                // Do not use parent-death SIGKILL: leader exit is the scenario
                // under test. Alarm bounds an abandoned fixture's lifetime.
                libc::alarm(6);
                if libc::write(descriptors[1], ready_pointer, 1) != 1 {
                    libc::_exit(126);
                }
                libc::close(descriptors[1]);
                loop {
                    libc::pause();
                }
            }
        }
        let read = unsafe { OwnedFd::from_raw_fd(descriptors[0]) };
        let write = unsafe { OwnedFd::from_raw_fd(descriptors[1]) };
        drop(write);
        let mut reader = File::from(read);
        let mut observed = [0u8; 1];
        reader.read_exact(&mut observed).unwrap();
        assert_eq!(observed, ready);
        let descendant = image(pid as u32).unwrap().unwrap();
        fs::write(
            marker,
            serde_json::to_vec(&serde_json::json!({
                "pid":descendant.pid,"birth":descendant.birth,"group":descendant.group,
                "parent":descendant.parent,
            }))
            .unwrap(),
        )
        .unwrap();
    }
    fn descendant_body(root: &Path) {
        assert!(children().unwrap().is_empty());
        assert_eq!(
            unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) },
            0
        );
        let mut evidence = Vec::new();
        let mut errors = Vec::new();
        for hold_output in [false, true] {
            let marker = root.join(if hold_output {
                "inherited.json"
            } else {
                "quiet.json"
            });
            let spawned = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", DESC_WORKER, "--nocapture", "--test-threads=1"])
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env("HOME", root)
                .env("TMPDIR", root)
                .env("CODEX_HOME", root.join("codex"))
                .env("CLAUDE_CONFIG_DIR", root.join("claude"))
                .env("INFRA_ONNX_DESCENDANT_MARKER", &marker)
                .env(
                    "INFRA_ONNX_DESCENDANT_HOLD",
                    if hold_output { "1" } else { "0" },
                )
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn();
            let child = match spawned {
                Ok(child) => child,
                Err(error) => {
                    errors.push(format!("fixture spawn: {error}"));
                    continue;
                }
            };
            let leader = child.id();
            let mut owned = Some(PendingWorker::new(child));
            let started = Instant::now();
            let output = super::pipes(
                owned.as_mut().unwrap(),
                b"{}\n",
                started + Duration::from_millis(500),
            );
            let finished =
                finish_worker(&mut owned, started + Duration::from_millis(900)).unwrap_or(false);
            let record = fs::read(&marker).and_then(|bytes| {
                serde_json::from_slice::<serde_json::Value>(&bytes).map_err(io::Error::other)
            });
            let mut alive = None;
            let mut identity_matches = false;
            let mut record_json = serde_json::Value::Null;
            match record {
                Ok(record) => {
                    if let (Some(pid), Some(birth), Some(group), Some(parent)) = (
                        record["pid"].as_u64(),
                        record["birth"].as_u64(),
                        record["group"].as_u64(),
                        record["parent"].as_u64(),
                    ) {
                        let until = Instant::now() + Duration::from_millis(200);
                        loop {
                            match image(pid as u32) {
                                Ok(Some(current))
                                    if current.birth == birth && current.group as u64 == group =>
                                {
                                    identity_matches =
                                        group == leader as u64 && parent == leader as u64;
                                    alive = Some(current.state != "Z");
                                }
                                Ok(None) => {
                                    identity_matches =
                                        group == leader as u64 && parent == leader as u64;
                                    alive = Some(false);
                                }
                                Ok(Some(_)) => {
                                    errors.push("descendant identity changed".into());
                                    break;
                                }
                                Err(error) => {
                                    errors.push(format!("descendant image: {error}"));
                                    break;
                                }
                            }
                            if alive == Some(false) || Instant::now() >= until {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(1));
                        }
                    } else {
                        errors.push("fixture marker has invalid identity".into());
                    }
                    record_json = record;
                }
                Err(error) => errors.push(format!("fixture marker: {error}")),
            }
            // Evidence precedes cleanup. As a private subreaper, this driver
            // owns orphaned descendants and can pin+reap each exact birth.
            let cleanup_until = Instant::now() + Duration::from_secs(2);
            let _ = finish_worker(&mut owned, cleanup_until);
            match children() {
                Ok(found) => {
                    for process in found.values() {
                        if let Err(error) = reap_owned(process, cleanup_until) {
                            errors
                                .push(format!("owned descendant cleanup {}: {error}", process.pid));
                        }
                    }
                }
                Err(error) => errors.push(format!("owned children: {error}")),
            }
            drop(owned);
            let remaining = children().unwrap().len();
            evidence.push(serde_json::json!({
                "hold_stdout":hold_output,"leader":leader,"descendant":record_json,
                "output_error":output.err().map(|error|error.reason),"finish_confirmed":finished,
                "identity_matches":identity_matches,"alive_after_finish":alive,"remaining_after_cleanup":remaining,
            }));
        }
        println!(
            "{DESC_MARKER} {}",
            serde_json::json!({"cases":evidence,"errors":errors})
        );
        assert!(
            errors.is_empty() && children().unwrap().is_empty(),
            "fixture must clean only its owned processes before assertions"
        );
        assert_eq!(evidence.len(), 2);
        for item in evidence {
            assert_eq!(
                item["identity_matches"], true,
                "fixture must prove the descendant shares its leader's group"
            );
            assert_eq!(item["finish_confirmed"], true);
            assert_eq!(
                item["alive_after_finish"], false,
                "a leader's natural exit must not release a still-live owned descendant"
            );
            assert_eq!(item["remaining_after_cleanup"], 0);
        }
    }
    #[test]
    fn exited_leader_descendants_are_retired_before_custody_release() {
        if let Some(root) = std::env::var_os(CHILD) {
            descendant_body(Path::new(&root));
        } else {
            isolated(DESC_TEST, DESC_MARKER);
        }
    }

    #[test]
    fn finished_io_panic_fails_after_retiring_real_child() {
        const NAME: &str = "onnx::cleanup_tests::finished_io_panic_fails_after_retiring_real_child";
        const OBSERVED: &str = "ONNX_IO_PANIC_CUSTODY_OBSERVED";
        let Some(root) = std::env::var_os(CHILD) else {
            isolated(NAME, OBSERVED);
            return;
        };
        assert!(children().unwrap().is_empty());
        let worker = ["/usr/bin/true", "/bin/true"]
            .into_iter()
            .find(|path| Path::new(path).is_file())
            .unwrap();
        let child = Command::new(worker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        let mut owned = Some(PendingWorker::new(child));
        let mut errors = Vec::new();
        match std::thread::Builder::new()
            .name("onnx-panic-fixture".into())
            .spawn(|| {
                panic!("controlled ONNX I/O thread panic");
            }) {
            Ok(thread) => owned.as_mut().unwrap().threads.push(thread),
            Err(error) => errors.push(format!("panic fixture thread: {error}")),
        }
        let result = finish_worker(&mut owned, Instant::now() + Duration::from_secs(2));
        let released = owned.is_none();
        let remaining_before_cleanup = children().unwrap().len();
        let cleanup_until = Instant::now() + Duration::from_secs(2);
        let _ = finish_worker(&mut owned, cleanup_until);
        for process in children().unwrap().values() {
            if let Err(error) = reap_owned(process, cleanup_until) {
                errors.push(format!("panic fixture cleanup: {error}"));
            }
        }
        drop(owned);
        let remaining = children().unwrap().len();
        println!(
            "{OBSERVED} {}",
            serde_json::json!({
                "error":result.as_ref().err().map(|error|error.reason),"released":released,
                "remaining_before_cleanup":remaining_before_cleanup,"remaining_after_cleanup":remaining,
                "errors":errors,"private_root":Path::new(&root).to_string_lossy(),
            })
        );
        assert!(
            errors.is_empty() && remaining == 0,
            "panic fixture cleanup must precede assertions"
        );
        assert_eq!(result.unwrap_err().reason, "worker-exit");
        assert!(released);
        assert_eq!(remaining_before_cleanup, 0);
    }
}

#[cfg(all(test, windows))]
mod threaded_exit_tests {
    use super::{PendingWorker, WindowsJob, failure, finish_worker, pipes, strict};
    use std::{
        os::windows::process::CommandExt,
        path::PathBuf,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    #[test]
    fn threaded_worker_requires_natural_zero_exit() {
        let root = tempfile::Builder::new()
            .prefix("infra-onnx-threaded-exit-")
            .tempdir()
            .unwrap();
        let system = std::env::var_os("SystemRoot")
            .expect("Windows must identify its native system directory");
        let command = PathBuf::from(&system).join("System32/cmd.exe");
        for code in [0, 7] {
            // /D disables AutoRun. Only cmd builtins run: consume one complete
            // request line, emit exact valid scores, then exit with this code.
            let script =
                format!(r#"set /p request=& <nul set /p ={{"scores":[0.25]}}& exit /b {code}"#);
            let child = Command::new(&command)
                .args(["/D", "/Q", "/C"])
                .raw_arg(script)
                .env_clear()
                .env("SystemRoot", &system)
                .env("ComSpec", &command)
                .env("HOME", root.path())
                .env("USERPROFILE", root.path())
                .env("TEMP", root.path())
                .env("TMP", root.path())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let mut owned = Some(PendingWorker::new(child));
            let deadline = Instant::now() + Duration::from_secs(2);
            let output = (|| {
                let worker = owned.as_mut().unwrap();
                worker.job = Some(
                    WindowsJob::attach(&worker.child).map_err(|_| failure("worker-unavailable"))?,
                );
                pipes(worker, b"{}\n", deadline - Duration::from_millis(100))
            })();
            // "worker-exit" covers a failed write, a failed read and a
            // nonzero exit alike; the observed status tells them apart.
            let status = owned
                .as_mut()
                .map(|worker| worker.child.try_wait().map_err(|error| error.to_string()));
            let retired = finish_worker(&mut owned, deadline);
            let recovered = if owned.is_some() {
                finish_worker(&mut owned, Instant::now() + Duration::from_secs(2))
            } else {
                retired
            };
            assert!(
                recovered.unwrap() && owned.is_none(),
                "test worker cleanup must precede status assertions"
            );
            if code == 0 {
                let raw = output.unwrap_or_else(|error| {
                    panic!(
                        "zero exit with full input and EOF must succeed: {error:?}, status {status:?}"
                    )
                });
                let value = strict::decode(&raw, super::RESPONSE, 16, 4096).unwrap();
                assert_eq!(value, serde_json::json!({"scores":[0.25]}));
            } else {
                assert_eq!(
                    output.unwrap_err().reason,
                    "worker-exit",
                    "valid scores from a nonzero native worker must be refused"
                );
            }
        }
    }
}
