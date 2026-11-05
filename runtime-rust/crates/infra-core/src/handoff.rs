//! Portable, source-bound standalone context handoffs, including v1/v2 transcripts.
use crate::{
    Result, RuntimeContext, RuntimeError,
    domain::{self, clock, json},
    fs, git, result_store, results,
};
use serde_json::{Value, json as object};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    path::{Path, PathBuf},
};
pub const INPUT_LIMIT: usize = 1024 * 1024;
pub const TRANSCRIPT_LIMIT: usize = 4 * INPUT_LIMIT;
pub const HANDOFF_LIMIT: usize = 6 * INPUT_LIMIT;
const MARKER: &str = "\n## Visible transcript\n\n<!-- context-handoff-transcript-v1 -->\n";
const ENCODED_MARKER: &str =
    "\n## Visible transcript (JSON string)\n\n<!-- context-handoff-transcript-v2 -->\n";
const FIELDS: [&str; 9] = [
    "goal",
    "summary",
    "constraints",
    "decisions",
    "progress",
    "verification",
    "open_questions",
    "next_steps",
    "files",
];
#[derive(Clone, Debug)]
pub struct SaveHandoff {
    pub input: Value,
    pub output: PathBuf,
    pub detail: String,
    pub topic: Option<String>,
    pub task_id: Option<String>,
    pub source_client: String,
    pub transcript: Option<PathBuf>,
}
/// Curated text carries neither secrets nor personal data, the policy the
/// memory runtime applies to everything an agent authors for later tasks;
/// refused in the retired words.
fn reject_sensitive(label: &str, text: &str) -> Result<()> {
    match domain::sensitive_label(text)? {
        Some(found) => Err(RuntimeError::policy(format!(
            "{found} detected; {label} not stored"
        ))),
        None => Ok(()),
    }
}

#[derive(Clone)]
pub struct HandoffService {
    pub context: RuntimeContext,
}
impl HandoffService {
    pub fn new(context: RuntimeContext) -> Self {
        Self { context }
    }
    pub fn validate_curated(&self, value: &Value, detail: &str) -> Result<Value> {
        let limit = match detail {
            "summary" => 8000,
            "topic" => 16000,
            "full" => 64000,
            _ => {
                return Err(RuntimeError::invalid(
                    "detail must be one of: summary, topic, full",
                ));
            }
        };
        let map = value
            .as_object()
            .ok_or_else(|| RuntimeError::invalid("--input must be one JSON object"))?;
        let joined = |keys: BTreeSet<&str>| keys.into_iter().collect::<Vec<_>>().join(", ");
        let unexpected = map
            .keys()
            .map(String::as_str)
            .filter(|key| !FIELDS[..9].contains(key))
            .collect::<BTreeSet<_>>();
        if !unexpected.is_empty() {
            return Err(RuntimeError::invalid(format!(
                "input has unexpected fields: {}",
                joined(unexpected)
            )));
        }
        let missing = ["goal", "summary", "next_steps"]
            .into_iter()
            .filter(|key| !map.contains_key(*key))
            .collect::<BTreeSet<_>>();
        if !missing.is_empty() {
            return Err(RuntimeError::invalid(format!(
                "input is missing required fields: {}",
                joined(missing)
            )));
        }
        let mut output = object!({});
        let mut authored = Vec::new();
        for key in &FIELDS[..9] {
            if ["goal", "summary"].contains(key) {
                let text = value[key]
                    .as_str()
                    .filter(|text| !results::clean(text).is_empty())
                    .ok_or_else(|| {
                        RuntimeError::invalid(format!("{key} must be a non-empty string"))
                    })?;
                let text = results::clean(text);
                output[key] = text.clone().into();
                authored.push(text);
            } else {
                let rows = map
                    .get(*key)
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                if map.contains_key(*key) && !value[key].is_array() {
                    return Err(RuntimeError::invalid(format!(
                        "{key} must be a list of non-empty strings"
                    )));
                }
                let rows = rows
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .filter(|text| !results::clean(text).is_empty())
                            .map(String::from)
                            .ok_or_else(|| {
                                RuntimeError::invalid(format!(
                                    "{key} must be a list of non-empty strings"
                                ))
                            })
                    })
                    .collect::<Result<Vec<_>>>()?;
                let rows = if *key == "files" {
                    rows
                } else {
                    rows.iter().map(|value| results::clean(value)).collect()
                };
                authored.extend(rows.clone());
                output[key] = object!(rows);
            }
        }
        if output["next_steps"].as_array().is_none_or(Vec::is_empty) {
            return Err(RuntimeError::invalid("next_steps must not be empty"));
        }
        let authored = authored.join("\n");
        if authored.chars().count() > limit {
            return Err(RuntimeError::invalid(format!(
                "curated context exceeds the {detail} limit of {limit} characters"
            )));
        }
        reject_sensitive("curated context", &authored)?;
        let transcript=regex::Regex::new(r"(?im)^\s*(?:user|assistant|system|developer|tool|prompt|response|reasoning|stdout|stderr|log)\s*:").map_err(|_|RuntimeError::invalid("Invalid handoff policy expression"))?;
        if transcript.is_match(&authored) {
            return Err(RuntimeError::policy(
                "curated context contains raw transcript-like data; replace it with a summary",
            ));
        }
        Ok(output)
    }
    /// A cited file as the handoff stores it, repository-relative; the
    /// directories above the repository are this machine's and never leave it.
    fn safe_relative(&self, value: &str) -> Result<(String, PathBuf)> {
        if value.is_empty()
            || value != value.trim()
            || value.contains('\\')
            || value.chars().any(|character| (character as u32) < 32)
        {
            return Err(RuntimeError::invalid(
                "file path must be a non-empty, trimmed repository-relative path",
            ));
        }
        let parts = value
            .split('/')
            .filter(|part| !part.is_empty() && *part != ".")
            .collect::<Vec<_>>();
        if value.starts_with('/')
            || value.starts_with("./")
            || parts.contains(&"..")
            || parts.first().is_some_and(|first| first.contains(':'))
        {
            return Err(RuntimeError::policy(format!(
                "file path escapes the repository: {value}"
            )));
        }
        static SENSITIVE: std::sync::OnceLock<std::result::Result<regex::Regex, regex::Error>> =
            std::sync::OnceLock::new();
        let sensitive = SENSITIVE
            .get_or_init(|| {
                regex::Regex::new(
                    r"(?i)^(?:\.git|\.env(?:\..*)?|secrets?|credentials?|id_[a-z0-9]+|[^/]+\.(?:pem|key|p12|pfx|jks|keystore))$",
                )
            })
            .as_ref()
            .map_err(|_| RuntimeError::invalid("Invalid handoff path expression"))?;
        if parts.is_empty() || parts.iter().any(|part| sensitive.is_match(part)) {
            return Err(RuntimeError::policy(format!(
                "file path is sensitive or invalid: {value}"
            )));
        }
        let relative = parts.join("/");
        let mut current = self.context.roots.project.clone();
        for part in &parts {
            current.push(part);
            if std::fs::symlink_metadata(&current).is_ok_and(|meta| meta.file_type().is_symlink()) {
                return Err(RuntimeError::policy(format!(
                    "file path contains a symlink: {value}"
                )));
            }
        }
        let path = self.context.roots.project.join(&relative);
        fs::admit_descendant(&self.context.roots.project, &path)?;
        Ok((relative, path))
    }
    pub fn fingerprint_files(&self, values: &[String]) -> Result<Vec<Value>> {
        if values.len() > 256 {
            return Err(RuntimeError::invalid(
                "at most 256 source files may be referenced",
            ));
        }
        let frame = git::GitFrame::discover(&self.context.roots.project)?;
        let mut seen = BTreeSet::new();
        let mut rows = Vec::new();
        for value in values {
            let (relative, path) = self.safe_relative(value)?;
            if !seen.insert(relative.clone()) {
                return Err(RuntimeError::invalid(format!(
                    "file paths must be unique: {relative}"
                )));
            }
            if let Some(frame) = &frame
                && !frame.ignored(std::slice::from_ref(&relative))?.is_empty()
            {
                return Err(RuntimeError::policy(format!(
                    "file path is ignored and cannot be included: {relative}"
                )));
            }
            let checksum = match std::fs::File::open(&path) {
                Ok(mut handle) => {
                    if !handle.metadata()?.is_file() {
                        return Err(RuntimeError::policy(format!(
                            "file path is not a regular file: {relative}"
                        )));
                    }
                    use sha2::{Digest, Sha256};
                    let mut checksum = Sha256::new();
                    let mut block = [0u8; 65536];
                    loop {
                        let count = handle.read(&mut block)?;
                        if count == 0 {
                            break;
                        }
                        checksum.update(&block[..count]);
                    }
                    Some(
                        checksum
                            .finalize()
                            .iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<String>(),
                    )
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            rows.push(object!({"path":relative,"sha256":checksum}));
        }
        Ok(rows)
    }
    fn git_value(&self, args: &[&str]) -> Option<String> {
        git::run(
            &self.context.roots.project,
            &git::strings(args),
            None,
            std::time::Duration::from_secs(2),
            65536,
        )
        .ok()
        .filter(|result| result.code == 0)
        .and_then(|result| String::from_utf8(result.stdout).ok())
        .map(|text| results::clean(&text))
    }
    fn transcript(&self, path: &Path) -> Result<Vec<u8>> {
        if !matches!(
            path.extension()
                .and_then(|value| value.to_str())
                .map(str::to_lowercase)
                .as_deref(),
            Some("md" | "markdown" | "txt")
        ) {
            return Err(RuntimeError::invalid(
                "Transcript must be a visible .txt or Markdown export",
            ));
        }
        no_link_parents(path)?;
        let raw = result_store::read_bounded(path, TRANSCRIPT_LIMIT)?
            .ok_or_else(|| RuntimeError::not_found("Transcript is unavailable"))?;
        let text = std::str::from_utf8(&raw)
            .map_err(|_| RuntimeError::invalid("Transcript must be UTF-8"))?;
        if results::clean(text).is_empty() {
            return Err(RuntimeError::invalid("Transcript must not be empty"));
        }
        if domain::secret_label(text)?.is_some() {
            return Err(RuntimeError::policy("Transcript contains a credential"));
        }
        let internal=regex::Regex::new(r#"(?i)"(?:type|role)"\s*:\s*"(?:system|developer|tool|reasoning|function_call|tool_call|response_item)""#).map_err(|_|RuntimeError::invalid("Invalid transcript expression"))?;
        if text
            .lines()
            .filter(|line| !results::clean(line).is_empty())
            .all(|line| line.trim_start().starts_with('{'))
            && internal.is_match(text)
        {
            return Err(RuntimeError::policy(
                "Transcript appears to contain internal reasoning or tool JSONL",
            ));
        }
        Ok(raw)
    }
    pub fn save(&self, input: SaveHandoff) -> Result<Value> {
        if !["codex", "claude", "cursor", "other"].contains(&input.source_client.as_str()) {
            return Err(RuntimeError::invalid(
                "source client must be one of: claude, codex, cursor, other",
            ));
        }
        let topic = input
            .topic
            .as_ref()
            .is_some_and(|topic| !results::clean(topic).is_empty());
        for (wrong, message) in [
            (
                input.detail == "topic" && !topic,
                "--topic is required when --detail topic",
            ),
            (
                input.detail != "topic" && input.topic.is_some(),
                "--topic is valid only when --detail topic",
            ),
            (
                input.detail == "full" && input.transcript.is_none(),
                "--transcript is required when --detail full",
            ),
            (
                input.detail != "full" && input.transcript.is_some(),
                "--transcript is valid only when --detail full",
            ),
        ] {
            if wrong {
                return Err(RuntimeError::invalid(message));
            }
        }
        // The labels go into the same tracked file as the curated fields and
        // follow the same policy: no secrets, no personal data.
        reject_sensitive(
            "topic and task ID",
            &input
                .topic
                .iter()
                .chain(input.task_id.iter())
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"),
        )?;
        let context = self.validate_curated(&input.input, &input.detail)?;
        let files = values(&context["files"])?;
        let fingerprints = self.fingerprint_files(&files)?;
        no_link_parents(&input.output)?;
        let output = fs::absolute(&input.output)?;
        if output.extension().and_then(|value| value.to_str()) != Some("md") {
            return Err(RuntimeError::invalid(
                "handoff output must use a .md extension",
            ));
        }
        if std::fs::symlink_metadata(&output).is_ok() {
            return Err(RuntimeError::conflict(format!(
                "handoff output already exists: {}",
                output.display()
            )));
        }
        if let Ok(relative) = output.strip_prefix(&self.context.roots.project)
            && relative
                .components()
                .next()
                .and_then(|value| value.as_os_str().to_str())
                .is_some_and(|name| {
                    [
                        "memory-bank",
                        "project-brain",
                        ".git",
                        ".agents",
                        ".claude",
                        ".cursor",
                        ".codex",
                        ".context-handoff",
                    ]
                    .contains(&name)
                })
        {
            return Err(RuntimeError::policy(
                "handoff output must be a standalone task artifact, outside memory and policy stores",
            ));
        }
        if self.context.roots.attached() && output.starts_with(&self.context.roots.state) {
            return Err(RuntimeError::policy(
                "handoff output must be a standalone task artifact, outside the accelerator state",
            ));
        }
        let transcript = input
            .transcript
            .as_ref()
            .map(|path| self.transcript(path))
            .transpose()?;
        let encoded = transcript
            .as_ref()
            .is_some_and(|bytes| bytes.contains(&b'\r'));
        let mut body = render_body(&context)?.into_bytes();
        if let Some(raw) = &transcript {
            body.extend_from_slice(if encoded {
                ENCODED_MARKER.as_bytes()
            } else {
                MARKER.as_bytes()
            });
            if encoded {
                let text = std::str::from_utf8(raw)
                    .map_err(|_| RuntimeError::invalid("Invalid transcript encoding"))?;
                body.extend_from_slice(
                    json::python_default(&object!(text), false, false, true)?.as_bytes(),
                );
            } else {
                body.extend_from_slice(raw);
            }
        }
        if body.len() > HANDOFF_LIMIT - INPUT_LIMIT {
            return Err(RuntimeError::invalid(
                "encoded handoff exceeds the size limit",
            ));
        }
        let metadata = object!({"schema_version":if encoded{2}else{1},"type":"context-handoff","created_at":clock::now().to_rfc3339(),"detail":input.detail,"topic":input.topic.map(|value|results::clean(&value)),"source_client":input.source_client,"task_id":input.task_id.map(|value|results::clean(&value)).filter(|value|!value.is_empty()),"repository":{"branch":self.git_value(&["branch","--show-current"]),"commit":self.git_value(&["rev-parse","HEAD"])},"files":fingerprints,"context":context,"body_sha256":result_store::sha(&body),"transcript":transcript.as_ref().map(|raw|object!({"sha256":result_store::sha(raw),"bytes":raw.len()}))});
        if let Some(label) = domain::secret_label(&format!(
            "{}\n{}",
            json::python_default(&metadata, false, false, true)?,
            String::from_utf8_lossy(&body)
        ))? {
            return Err(RuntimeError::policy(format!(
                "possible {label} detected; handoff not stored"
            )));
        }
        let parent = output
            .parent()
            .ok_or_else(|| RuntimeError::invalid("Handoff output has no parent"))?;
        fs::ensure_directory(parent, true)?;
        no_link_parents(&output)?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        let mut handle = options.open(&output).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                RuntimeError::conflict(format!(
                    "handoff output already exists: {}",
                    output.display()
                ))
            } else {
                error.into()
            }
        })?;
        handle.write_all(b"---\n")?;
        handle.write_all(json::python_default(&metadata, true, false, true)?.as_bytes())?;
        handle.write_all(b"\n---\n")?;
        handle.write_all(&body)?;
        handle.flush()?;
        handle.sync_all()?;
        Ok(
            object!({"output":output,"detail":metadata["detail"],"files":fingerprints.len(),"transcript":transcript.is_some()}),
        )
    }
    /// A handoff's metadata, curated context and transcript, refused in the
    /// retired loader's words and order.
    fn decode(&self, raw: &[u8]) -> Result<(Value, Value, Option<Vec<u8>>)> {
        let invalid = |message: &str| RuntimeError::invalid(message.to_string());
        let rest = raw
            .strip_prefix(b"---\n")
            .ok_or_else(|| invalid("handoff is missing JSON frontmatter"))?;
        let split = rest
            .windows(5)
            .position(|window| window == b"\n---\n")
            .ok_or_else(|| invalid("handoff has invalid JSON frontmatter"))?;
        let body = &rest[split + 5..];
        let metadata = std::str::from_utf8(&rest[..split])
            .ok()
            .and_then(|text| json::parse(text).ok())
            .ok_or_else(|| invalid("handoff has invalid JSON frontmatter"))?;
        if !metadata.is_object() {
            return Err(invalid("handoff frontmatter must be an object"));
        }
        let required = [
            "schema_version",
            "type",
            "created_at",
            "detail",
            "topic",
            "source_client",
            "task_id",
            "repository",
            "files",
            "context",
            "body_sha256",
            "transcript",
        ];
        if !result_store::exact(&metadata, &required) {
            return Err(invalid("handoff frontmatter has an unsupported schema"));
        }
        if !matches!(metadata["schema_version"].as_u64(), Some(1 | 2))
            || metadata["type"] != "context-handoff"
        {
            return Err(invalid("handoff schema version or type is unsupported"));
        }
        let detail = metadata["detail"]
            .as_str()
            .ok_or_else(|| invalid("handoff detail is invalid"))?;
        let topic = &metadata["topic"];
        if detail == "topic" && !topic.as_str().is_some_and(|text| !text.trim().is_empty())
            || detail != "topic" && !topic.is_null()
        {
            return Err(invalid("handoff topic is invalid"));
        }
        if !metadata["created_at"]
            .as_str()
            .is_some_and(domain::valid_datetime)
        {
            return Err(invalid("handoff creation time is invalid"));
        }
        if !matches!(
            metadata["source_client"].as_str(),
            Some("codex" | "claude" | "cursor" | "other")
        ) {
            return Err(invalid("handoff source client is invalid"));
        }
        if !metadata["task_id"].is_null() && !metadata["task_id"].is_string() {
            return Err(invalid("handoff task ID is invalid"));
        }
        reject_sensitive(
            "topic and task ID",
            &[&metadata["topic"], &metadata["task_id"]]
                .into_iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n"),
        )?;
        let repository = &metadata["repository"];
        if !result_store::exact(repository, &["branch", "commit"])
            || ["branch", "commit"]
                .iter()
                .any(|key| !repository[*key].is_null() && !repository[*key].is_string())
        {
            return Err(invalid("handoff repository provenance is invalid"));
        }
        let fingerprints = metadata["files"]
            .as_array()
            .filter(|rows| {
                rows.iter().all(|row| {
                    result_store::exact(row, &["path", "sha256"])
                        && row["path"].is_string()
                        && (row["sha256"].is_null()
                            || row["sha256"]
                                .as_str()
                                .is_some_and(|digest| result_store::hex(digest, 64)))
                })
            })
            .ok_or_else(|| invalid("handoff file fingerprints are invalid"))?;
        if !metadata["context"].is_object() {
            return Err(invalid("handoff curated context is invalid"));
        }
        let context = self.validate_curated(&metadata["context"], detail)?;
        // Paths compare as Python's PurePosixPath spells them.
        let posix = |path: &str| {
            let joined = path
                .split('/')
                .filter(|part| !part.is_empty() && *part != ".")
                .collect::<Vec<_>>()
                .join("/");
            match (path.starts_with('/'), joined.is_empty()) {
                (true, _) => format!("/{joined}"),
                (false, true) => ".".to_string(),
                (false, false) => joined,
            }
        };
        let cited = values(&context["files"])?
            .iter()
            .map(|path| posix(path))
            .collect::<Vec<_>>();
        if fingerprints
            .iter()
            .map(|row| row["path"].as_str().unwrap_or(""))
            .ne(cited.iter().map(String::as_str))
        {
            return Err(invalid(
                "handoff fingerprints do not match its file references",
            ));
        }
        let stated = &metadata["transcript"];
        if (detail == "full") != !stated.is_null() {
            return Err(invalid("only a full handoff must contain a transcript"));
        }
        let mut expected = render_body(&context)?.into_bytes();
        let transcript = if stated.is_null() {
            None
        } else if result_store::exact(stated, &["sha256", "bytes"])
            && stated["sha256"]
                .as_str()
                .is_some_and(|digest| result_store::hex(digest, 64))
            && stated["bytes"]
                .as_u64()
                .is_some_and(|bytes| bytes > 0 && bytes as usize <= TRANSCRIPT_LIMIT)
        {
            let encoded = metadata["schema_version"] == 2;
            expected.extend_from_slice(if encoded {
                ENCODED_MARKER.as_bytes()
            } else {
                MARKER.as_bytes()
            });
            let stored = body
                .strip_prefix(expected.as_slice())
                .ok_or_else(|| invalid("handoff transcript marker is missing"))?;
            let transcript = if encoded {
                let visible = std::str::from_utf8(stored)
                    .ok()
                    .and_then(|text| json::parse(text).ok())
                    .and_then(|value| value.as_str().map(String::from))
                    .ok_or_else(|| invalid("handoff transcript encoding is invalid"))?;
                if let Some(label) = domain::secret_label(&visible)? {
                    return Err(RuntimeError::policy(format!(
                        "possible {label} detected; transcript not stored"
                    )));
                }
                expected.extend_from_slice(
                    json::python_default(&object!(visible.clone()), false, false, true)?.as_bytes(),
                );
                visible.into_bytes()
            } else {
                expected.extend_from_slice(stored);
                stored.to_vec()
            };
            if stated["bytes"].as_u64() != Some(transcript.len() as u64)
                || stated["sha256"] != result_store::sha(&transcript)
            {
                return Err(invalid("handoff transcript digest does not match"));
            }
            Some(transcript)
        } else {
            return Err(invalid("handoff transcript metadata is invalid"));
        };
        if expected != body {
            return Err(invalid("handoff body does not match its curated context"));
        }
        if metadata["body_sha256"] != result_store::sha(body) {
            return Err(invalid("handoff body digest does not match"));
        }
        Ok((metadata, context, transcript))
    }
    pub fn load(&self, path: &Path, include_transcript: bool) -> Result<Value> {
        no_link_parents(path)?;
        let raw = result_store::read_bounded(path, HANDOFF_LIMIT)?
            .ok_or_else(|| RuntimeError::not_found("Handoff is unavailable"))?;
        let mut normalized = Vec::with_capacity(raw.len());
        let mut index = 0;
        while index < raw.len() {
            if raw[index] == b'\r' && raw.get(index + 1) == Some(&b'\n') {
                index += 1;
            }
            normalized.push(raw[index]);
            index += 1;
        }
        let readings = if raw.starts_with(b"---\r\n") {
            vec![normalized.as_slice(), raw.as_slice()]
        } else {
            vec![raw.as_slice(), normalized.as_slice()]
        };
        let mut parsed = None;
        let mut failure = None;
        for reading in readings {
            match self.decode(reading) {
                Ok(value) => {
                    parsed = Some(value);
                    break;
                }
                Err(error) => {
                    if failure.is_none() {
                        failure = Some(error);
                    }
                }
            }
        }
        let (metadata, context, transcript) = parsed
            .ok_or_else(|| failure.unwrap_or_else(|| RuntimeError::invalid("Invalid handoff")))?;
        let branch = self.git_value(&["branch", "--show-current"]);
        let commit = self.git_value(&["rev-parse", "HEAD"]);
        let mut drift = Vec::new();
        if metadata["repository"]["branch"] != object!(branch) {
            drift.push(object!({"kind":"branch","saved":metadata["repository"]["branch"],"current":branch}));
        }
        if metadata["repository"]["commit"] != object!(commit) {
            drift.push(object!({"kind":"commit","saved":metadata["repository"]["commit"],"current":commit}));
        }
        let files = self.fingerprint_files(&values(&context["files"])?)?;
        for (saved, current) in metadata["files"]
            .as_array()
            .into_iter()
            .flatten()
            .zip(&files)
        {
            if current["sha256"].is_null() || saved["sha256"] != current["sha256"] {
                drift.push(object!({"kind":"file","path":current["path"],"saved":saved["sha256"],"current":current["sha256"]}));
            }
        }
        let mut public = metadata.clone();
        public
            .as_object_mut()
            .ok_or_else(|| RuntimeError::invalid("Invalid handoff metadata"))?
            .remove("context");
        let mut result = object!({"input":std::fs::canonicalize(path)?,"metadata":public,"context":context,"drift":drift});
        if include_transcript && let Some(raw) = transcript {
            result["transcript"] = String::from_utf8(raw)
                .map_err(|_| RuntimeError::invalid("Invalid transcript encoding"))?
                .into();
        }
        Ok(result)
    }
}
fn values(value: &Value) -> Result<Vec<String>> {
    value
        .as_array()
        .ok_or_else(|| RuntimeError::invalid("Expected a string list"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(String::from)
                .ok_or_else(|| RuntimeError::invalid("Expected text"))
        })
        .collect()
}
fn no_link_parents(path: &Path) -> Result<()> {
    let path = fs::absolute(path)?;
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if let Ok(metadata) = std::fs::symlink_metadata(&current)
            && fs::linked_at(&current, &metadata)?
        {
            return Err(RuntimeError::policy("Symlink handoff paths are refused"));
        }
    }
    Ok(())
}
pub fn render_body(context: &Value) -> Result<String> {
    let mut output = format!(
        "# Context handoff\n\n## Goal\n\n{}\n\n## Summary\n\n{}\n\n",
        context["goal"].as_str().unwrap_or(""),
        context["summary"].as_str().unwrap_or("")
    );
    for (key, title) in [
        ("constraints", "Constraints"),
        ("decisions", "Decisions"),
        ("progress", "Progress"),
        ("verification", "Verification"),
        ("open_questions", "Open questions"),
        ("next_steps", "Next steps"),
        ("files", "Files"),
    ] {
        let values = values(&context[key])?;
        let rendered = if values.is_empty() {
            "- None".into()
        } else {
            values
                .iter()
                .map(|value| format!("- {value}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        output.push_str(&format!(
            "## {title}\n\n{rendered}{}",
            if key == "files" { "\n" } else { "\n\n" }
        ));
    }
    Ok(output)
}
