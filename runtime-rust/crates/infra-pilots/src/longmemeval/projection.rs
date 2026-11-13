use crate::strict;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};
pub const PROFILE: &str = "native-fts5/public-states-text-projection-v1";
pub const CONTEXT_BYTES: usize = 262144;
#[derive(Debug, Clone, Copy)]
pub struct ProjectionError(pub &'static str);
impl std::fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for ProjectionError {}
macro_rules! refuse {
    ($guard:literal) => {
        return Err(ProjectionError($guard).into())
    };
}
fn schema_error(error: anyhow::Error) -> anyhow::Error {
    if error.is::<ProjectionError>() {
        return error;
    }
    let text = error.to_string();
    let guard = if text.starts_with("duplicate_key") {
        "duplicate_key"
    } else if [
        "input_schema",
        "input_bound",
        "input_encoding",
        "ambiguous_alias",
        "screenshot_reference",
    ]
    .contains(&text.as_str())
    {
        match text.as_str() {
            "input_schema" => "input_schema",
            "input_bound" => "input_bound",
            "input_encoding" => "input_encoding",
            "ambiguous_alias" => "ambiguous_alias",
            _ => "screenshot_reference",
        }
    } else {
        "input_encoding"
    };
    ProjectionError(guard).into()
}

#[derive(Debug, Clone)]
pub struct Projection {
    pub trajectory_id: String,
    pub relative: String,
    pub body: Vec<u8>,
    pub sha256: String,
    pub input_sha256: String,
    pub states: usize,
}
fn admit(v: &Value) -> Result<()> {
    let mut pending = vec![(v, 0)];
    let mut nodes = 0;
    while let Some((v, d)) = pending.pop() {
        nodes += 1;
        if nodes > 16000 || d > 8 {
            refuse!("input_bound");
        }
        match v {
            Value::Null => (),
            Value::Number(n) if n.as_u64().is_some_and(|n| n <= 2147483647) => (),
            Value::String(s) if s.chars().count() <= 65536 => (),
            Value::Object(m) if m.len() <= 16 => pending.extend(m.values().map(|v| (v, d + 1))),
            Value::Array(a) if a.len() <= 128 => pending.extend(a.iter().map(|v| (v, d + 1))),
            _ => refuse!("input_schema"),
        }
    }
    if strict::wire(v)?.len() > 262144 {
        refuse!("input_bound");
    }
    Ok(())
}
pub fn decode_trajectory(raw: &[u8]) -> Result<Value> {
    let v = infra_core::strict_json::parse(raw, 262144, 8, 16000).map_err(
        |error| -> anyhow::Error {
            let guard = match error.code.as_deref() {
                Some(
                    "strict-json:byte-limit" | "strict-json:depth-limit" | "strict-json:node-limit",
                ) => "input_bound",
                Some("strict-json:duplicate-key") => "duplicate_key",
                Some("strict-json:nonfinite") => "input_schema",
                _ => "input_encoding",
            };
            ProjectionError(guard).into()
        },
    )?;
    admit(&v)?;
    Ok(v)
}
fn nullable(v: Option<&Value>, max: usize) -> Result<Value> {
    match v {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(v) => Ok(strict::text(v, max, false, true)?.into()),
    }
}

#[cfg(test)]
mod transport_guard_tests {
    use super::{ProjectionError, decode_trajectory};

    fn guard(raw: &[u8]) -> &'static str {
        let error = decode_trajectory(raw).unwrap_err();
        error
            .downcast_ref::<ProjectionError>()
            .expect("transport refusals must retain the fixed projection guard")
            .0
    }

    #[test]
    fn nonfinite_numbers_are_schema_refusals() {
        for raw in [
            br#"{"step":NaN}"#.as_slice(),
            br#"{"step":Infinity}"#.as_slice(),
            br#"{"step":-Infinity}"#.as_slice(),
            br#"{"step":1e400}"#.as_slice(),
        ] {
            assert_eq!(guard(raw), "input_schema");
        }
        assert_eq!(decode_trajectory(br#"{"step":1}"#).unwrap()["step"], 1);
    }

    #[test]
    fn malformed_json_and_invalid_utf8_are_encoding_refusals() {
        for raw in [
            b"{".as_slice(),
            br#"{"step":}"#.as_slice(),
            br#"{"step":1} trailing"#.as_slice(),
            &[0xff],
        ] {
            assert_eq!(guard(raw), "input_encoding");
        }
    }

    #[test]
    fn duplicate_and_resource_guards_keep_their_specific_labels() {
        assert_eq!(guard(br#"{"a":1,"\u0061":2}"#), "duplicate_key");
        assert_eq!(guard(&vec![b' '; 262145]), "input_bound");
        let depth = format!("{}0{}", "[".repeat(9), "]".repeat(9));
        assert_eq!(guard(depth.as_bytes()), "input_bound");
        let nodes = format!("[{}]", vec!["0"; 16000].join(","));
        assert_eq!(guard(nodes.as_bytes()), "input_bound");
    }
}
pub fn project_trajectory(value: &Value) -> Result<Projection> {
    project_inner(value).map_err(schema_error)
}
fn project_inner(value: &Value) -> Result<Projection> {
    admit(value)?;
    let raw = strict::wire(value)?;
    let v = decode_trajectory(&raw)?;
    strict::keys(
        &v,
        &["id", "goal", "start_url", "outcome", "states"],
        &["id", "goal", "start_url", "states"],
    )?;
    let id = strict::text(&v["id"], 256, true, false)?.to_owned();
    let goal = strict::text(&v["goal"], 8192, false, true)?;
    let start = strict::text(&v["start_url"], 4096, true, false)?;
    let outcome = nullable(v.get("outcome"), 8192)?;
    let states = v["states"].as_array().context("input_schema")?;
    if states.is_empty() || states.len() > 128 {
        refuse!("input_schema");
    }
    let opaque = strict::sha(id.as_bytes());
    let mut lines = vec![
        format!("# Recorded trajectory {opaque}"),
        "".into(),
        format!("Goal: {}", serde_json::to_string(goal)?),
        format!("Start URL: {}", serde_json::to_string(start)?),
        format!("Outcome: {outcome}"),
    ];
    for (i, s) in states.iter().enumerate() {
        strict::keys(
            s,
            &[
                "url",
                "action",
                "thought",
                "thoughts",
                "accessibility_tree",
                "text",
                "screenshot",
                "step",
                "state_index",
            ],
            &["url", "screenshot"],
        )?;
        for (a, b) in [
            ("thought", "thoughts"),
            ("accessibility_tree", "text"),
            ("step", "state_index"),
        ] {
            if s.get(a).is_some() && s.get(b).is_some() {
                refuse!("ambiguous_alias");
            }
        }
        let step = s
            .get("step")
            .or_else(|| s.get("state_index"))
            .map(|v| {
                v.as_u64()
                    .filter(|n| *n <= 2147483647)
                    .context("input_schema")
            })
            .transpose()?
            .unwrap_or(i as u64);
        let url = strict::text(&s["url"], 4096, true, false)?;
        let action = nullable(s.get("action"), 8192)?;
        let thought = nullable(s.get("thought").or_else(|| s.get("thoughts")), 8192)?;
        let recorded = strict::text(
            s.get("accessibility_tree")
                .or_else(|| s.get("text"))
                .context("input_schema")?,
            65536,
            false,
            true,
        )?;
        let reference = strict::text(&s["screenshot"], 1024, true, false)?;
        strict::path(reference, 1024).map_err(|_| anyhow::anyhow!("screenshot_reference"))?;
        lines.extend([
            "".into(),
            format!("## Recorded state {i} (step {step})"),
            format!("URL: {}", serde_json::to_string(url)?),
            format!("Arrival action: {action}"),
            format!("Thought: {thought}"),
            format!(
                "Screenshot reference (not ingested): {}",
                serde_json::to_string(reference)?
            ),
            "".into(),
            "Recorded text:".into(),
            "".into(),
            recorded.into(),
        ]);
    }
    let body = (lines.join("\n") + "\n").into_bytes();
    if body.len() > 100000 {
        refuse!("input_bound");
    }
    Ok(Projection {
        trajectory_id: id,
        relative: format!("specs/longmemeval/trajectory-{opaque}.md"),
        sha256: strict::sha(&body),
        body,
        input_sha256: strict::sha(&raw),
        states: states.len(),
    })
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NativeMetadata {
    pub state: String,
    pub warnings: bool,
    pub has_evidence: bool,
    pub context_bytes: usize,
    pub context_chars: usize,
    pub query_image_ignored: bool,
    pub elapsed_seconds: Option<f64>,
}
#[derive(Debug, Clone)]
pub struct NativeRequest {
    pub query: String,
    pub task_id: String,
    pub limit: usize,
    pub ephemeral: bool,
    pub capsule_profile: &'static str,
    pub host: &'static str,
    pub entry_point: &'static str,
    pub allow_unprovisioned: bool,
}
pub struct NativeResponse {
    pub text: String,
    pub has_evidence: bool,
    pub warnings: bool,
    pub sanitized_empty: bool,
}
/// Trusted production implementation owns sanitization, privacy/policy gates, index refresh,
/// full capsule assembly, excerpt rendering and render-trace recording.
pub trait NativeRuntime: Send {
    fn refresh_and_render(
        &mut self,
        repository: &Path,
        request: NativeRequest,
    ) -> Result<NativeResponse>;
}
#[derive(Clone, PartialEq, Eq)]
struct Image {
    dev: u64,
    ino: u64,
    mode: u32,
    nlink: u64,
    size: u64,
    mtime: i64,
    mtime_ns: i64,
    ctime: i64,
    ctime_ns: i64,
}
fn image(m: &fs::Metadata) -> Image {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Image {
            dev: m.dev(),
            ino: m.ino(),
            mode: m.mode(),
            nlink: m.nlink(),
            size: m.size(),
            mtime: m.mtime(),
            mtime_ns: m.mtime_nsec(),
            ctime: m.ctime(),
            ctime_ns: m.ctime_nsec(),
        }
    }
    #[cfg(not(unix))]
    {
        Image {
            dev: 0,
            ino: 0,
            mode: 0,
            nlink: 1,
            size: m.len(),
            mtime: 0,
            mtime_ns: 0,
            ctime: 0,
            ctime_ns: 0,
        }
    }
}
fn same_node(a: &Image, b: &Image) -> bool {
    a.dev == b.dev && a.ino == b.ino && a.mode == b.mode
}
fn close_file(file: File) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::IntoRawFd;
        let fd = file.into_raw_fd();
        if unsafe { libc::close(fd) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        drop(file);
        Ok(())
    }
}
#[cfg(unix)]
fn open_at(dir: &File, name: &str, flags: i32, mode: u32) -> Result<File> {
    use std::{
        ffi::CString,
        os::fd::{AsRawFd, FromRawFd},
    };
    let name = CString::new(name)?;
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC,
            mode,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
pub struct NativeProjectionMemory<R: NativeRuntime> {
    repository: PathBuf,
    runtime: R,
    directories: Vec<Option<File>>,
    directory_images: Vec<Image>,
    documents: BTreeMap<String, Projection>,
    file_ids: BTreeMap<String, Image>,
    bytes: usize,
    queries: usize,
    closed: bool,
    fenced: bool,
    last: Option<NativeMetadata>,
    custody_state: &'static str,
    custody_stage: Option<&'static str>,
}
impl<R: NativeRuntime> NativeProjectionMemory<R> {
    pub fn open(repository: &Path, runtime: R) -> Result<Self> {
        if !repository.is_absolute()
            || !repository
                .join("project-brain/config/runtime.json")
                .is_file()
            || !repository.join("project-brain/schemas").is_dir()
        {
            refuse!("native_configuration");
        }
        #[cfg(not(unix))]
        {
            let _ = runtime;
            refuse!("native_filesystem_profile");
        }
        #[cfg(unix)]
        {
            use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
            let root = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(repository)?;
            let specs = open_at(
                &root,
                "specs",
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW,
                0,
            )?;
            let name = std::ffi::CString::new("longmemeval")?;
            if unsafe { libc::mkdirat(specs.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let source = open_at(
                &specs,
                "longmemeval",
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW,
                0,
            )?;
            specs.sync_all()?;
            let images = [&root, &specs, &source]
                .iter()
                .map(|f| f.metadata().map(|m| image(&m)))
                .collect::<std::io::Result<Vec<_>>>()?;
            Ok(Self {
                repository: repository.into(),
                runtime,
                directories: vec![Some(root), Some(specs), Some(source)],
                directory_images: images,
                documents: BTreeMap::new(),
                file_ids: BTreeMap::new(),
                bytes: 0,
                queries: 0,
                closed: false,
                fenced: false,
                last: None,
                custody_state: "unobserved",
                custody_stage: None,
            })
        }
    }
    fn ready(&self) -> Result<()> {
        if self.closed || self.fenced {
            refuse!("instance_unavailable");
        }
        Ok(())
    }
    fn custody(&mut self) -> Result<()> {
        self.ready()?;
        let mut stage = "directory_stat";
        let result = (|| -> Result<()> {
            for (i, p) in [
                &self.repository,
                self.repository.join("specs").as_path(),
                self.repository.join("specs/longmemeval").as_path(),
            ]
            .into_iter()
            .enumerate()
            {
                stage = "directory_stat";
                let m = fs::symlink_metadata(p)?;
                stage = "directory_guard";
                let held = self.directories[i]
                    .as_ref()
                    .context("source_changed")?
                    .metadata()?;
                if !m.is_dir()
                    || m.file_type().is_symlink()
                    || !same_node(&image(&m), &image(&held))
                    || !same_node(&image(&held), &self.directory_images[i])
                {
                    refuse!("source_changed");
                }
            }
            let mut expected = self
                .documents
                .values()
                .map(|p| p.relative.rsplit('/').next().unwrap().to_string())
                .collect::<BTreeSet<_>>();
            stage = "entry_scan";
            for e in fs::read_dir(self.repository.join("specs/longmemeval"))? {
                let n = e?.file_name().to_string_lossy().into_owned();
                if !expected.remove(&n) {
                    refuse!("source_changed");
                }
            }
            stage = "missing_entries_guard";
            if !expected.is_empty() {
                refuse!("source_changed");
            }
            for (id, p) in &self.documents {
                stage = "file_open";
                #[cfg(unix)]
                let mut file = open_at(
                    self.directories[2].as_ref().context("source_changed")?,
                    p.relative.rsplit('/').next().unwrap(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK,
                    0,
                )?;
                #[cfg(not(unix))]
                let mut file = File::open(self.repository.join(&p.relative))?;
                stage = "file_stat_before";
                let m = file.metadata()?;
                let before = image(&m);
                stage = "file_shape_guard";
                if !m.is_file() || before.nlink != 1 || before.size != p.body.len() as u64 {
                    refuse!("source_changed");
                }
                stage = "file_identity_guard";
                if !same_node(&before, &self.file_ids[id]) {
                    refuse!("source_changed");
                }
                stage = "file_read";
                let mut raw = vec![];
                std::io::Read::by_ref(&mut file)
                    .take(100001)
                    .read_to_end(&mut raw)?;
                stage = "file_stat_after";
                let after = image(&file.metadata()?);
                let ok =
                    before == after && raw.len() == p.body.len() && strict::sha(&raw) == p.sha256;
                stage = "file_close";
                close_file(file)?;
                if !ok {
                    stage = "file_image_guard";
                    refuse!("source_changed");
                }
            }
            Ok(())
        })();
        if result.is_err() {
            self.fenced = true;
            self.custody_state = "stage_reported";
            self.custody_stage = Some(stage);
            refuse!("source_changed");
        }
        Ok(())
    }
    pub fn insert(&mut self, trajectory: &Value) -> Result<()> {
        let p = project_trajectory(trajectory)?;
        self.custody()?;
        if let Some(old) = self.documents.get(&p.trajectory_id) {
            if old.input_sha256 == p.input_sha256 {
                return Ok(());
            }
            refuse!("conflicting_identity");
        }
        if self.documents.len() >= 500 || self.bytes + p.body.len() > 33554432 {
            refuse!("corpus_bound");
        }
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
            #[cfg(unix)]
            let mut file = open_at(
                self.directories[2]
                    .as_ref()
                    .context("instance_unavailable")?,
                p.relative.rsplit('/').next().unwrap(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW,
                0o600,
            )?;
            #[cfg(not(unix))]
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(self.repository.join(&p.relative))?;
            file.write_all(&p.body)?;
            file.sync_all()?;
            self.directories[2]
                .as_ref()
                .context("instance_unavailable")?
                .sync_all()?;
            let id = image(&file.metadata()?);
            self.bytes += p.body.len();
            self.file_ids.insert(p.trajectory_id.clone(), id);
            self.documents.insert(p.trajectory_id.clone(), p);
            close_file(file)?;
            Ok(())
        }));
        match caught {
            Ok(result) => {
                if result.is_err() {
                    self.fenced = true;
                }
                result
            }
            Err(payload) => {
                self.fenced = true;
                std::panic::resume_unwind(payload)
            }
        }
    }
    pub fn query(&mut self, query: &str, query_image: Option<&str>) -> Result<Vec<Value>> {
        strict::text(&json!(query), 8192, true, true)?;
        if let Some(image) = query_image {
            strict::text(&json!(image), 4096, true, false)?;
        }
        self.ready()?;
        if self.queries >= 4096 {
            refuse!("query_bound");
        }
        self.queries += 1;
        let start = Instant::now();
        self.last = Some(NativeMetadata {
            state: "failed".into(),
            query_image_ignored: query_image.is_some(),
            ..NativeMetadata::default()
        });
        let caught =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Vec<Value>> {
                self.custody()?;
                let response = self.runtime.refresh_and_render(
                    &self.repository,
                    NativeRequest {
                        query: query.into(),
                        task_id: format!("BENCH-{:04}", self.queries),
                        limit: 3,
                        ephemeral: true,
                        capsule_profile: "full",
                        host: "cli",
                        entry_point: "refresh",
                        allow_unprovisioned: true,
                    },
                )?;
                if response.text.len() > CONTEXT_BYTES {
                    refuse!("context_bound");
                }
                let meta = self.last.as_mut().unwrap();
                meta.warnings = response.warnings;
                meta.has_evidence = response.has_evidence;
                meta.state = if response.sanitized_empty {
                    "sanitized_empty"
                } else if response.warnings {
                    "refresh_failed"
                } else if response.has_evidence {
                    "delivered"
                } else {
                    "no_evidence"
                }
                .into();
                if response.sanitized_empty || response.warnings || !response.has_evidence {
                    return Ok(vec![]);
                }
                meta.context_bytes = response.text.len();
                meta.context_chars = response.text.chars().count();
                Ok(vec![json!({"type":"text","value":response.text})])
            }));
        let mut panic = None;
        let result = match caught {
            Ok(result) => result,
            Err(payload) => {
                panic = Some(payload);
                Err(anyhow::anyhow!("native_cancelled"))
            }
        };
        if result
            .as_ref()
            .err()
            .is_some_and(|e| e.is::<super::core_runtime::CleanupFailure>())
        {
            self.last.as_mut().unwrap().state = "cleanup_failed".into();
            self.last.as_mut().unwrap().context_bytes = 0;
            self.last.as_mut().unwrap().context_chars = 0;
        }
        self.last.as_mut().unwrap().elapsed_seconds = Some(start.elapsed().as_secs_f64());
        if let Some(payload) = panic {
            std::panic::resume_unwind(payload);
        }
        result
    }
    pub fn post_query_hook(&self) -> Value {
        let Some(m) = &self.last else {
            return json!({"profile":PROFILE,"state":"not_queried"});
        };
        json!({"profile":PROFILE,"state":m.state,"queries":self.queries,"trajectories":self.documents.len(),"projection_bytes":self.bytes,"context_bytes":m.context_bytes,"context_chars":m.context_chars,"images_ingested":0,"query_image_ignored":m.query_image_ignored,"provider_tokens":null,"provider_tokens_complete":false,"elapsed_seconds":m.elapsed_seconds})
    }
    pub fn custody_failure_observation(&self) -> Value {
        json!({"profile":"longmemeval/custody-failure-stage-v1","scope":"last_caught_custody_body_failure","observation_state":self.custody_state,"stage":self.custody_stage,"root_cause":null,"provider_tokens":null,"usd_micros":null})
    }
    pub fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        let mut first = None;
        for file in self.directories.iter_mut().rev() {
            if let Some(file) = file.take()
                && let Err(e) = close_file(file)
                && first.is_none()
            {
                first = Some(e);
            }
        }
        match first {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
    pub fn repository(&self) -> &Path {
        &self.repository
    }
}
impl<R: NativeRuntime> Drop for NativeProjectionMemory<R> {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
