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
            _ => return Err(RuntimeError::invalid("Invalid handoff detail")),
        };
        let map = value
            .as_object()
            .ok_or_else(|| RuntimeError::invalid("Handoff input must be one object"))?;
        if map.keys().any(|key| !FIELDS[..9].contains(&key.as_str()))
            || ["goal", "summary", "next_steps"]
                .iter()
                .any(|key| !map.contains_key(*key))
        {
            return Err(RuntimeError::invalid(
                "Handoff contains missing or unknown curated fields",
            ));
        }
        let mut output = object!({});
        let mut authored = Vec::new();
        for key in &FIELDS[..9] {
            if ["goal", "summary"].contains(key) {
                let text = value[key]
                    .as_str()
                    .filter(|text| !results::clean(text).is_empty())
                    .ok_or_else(|| RuntimeError::invalid(format!("{key} must be nonempty text")))?;
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
                    return Err(RuntimeError::invalid(format!("{key} must be a list")));
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
                                    "{key} must contain nonempty strings"
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
            return Err(RuntimeError::invalid(
                "Curated context exceeds its detail limit",
            ));
        }
        domain::guard_shared_text("curated context", std::slice::from_ref(&authored))?;
        let transcript=regex::Regex::new(r"(?im)^\s*(?:user|assistant|system|developer|tool|prompt|response|reasoning|stdout|stderr|log)\s*:").map_err(|_|RuntimeError::invalid("Invalid handoff policy expression"))?;
        if transcript.is_match(&authored) {
            return Err(RuntimeError::policy(
                "Curated context contains raw transcript-like data",
            ));
        }
        Ok(output)
    }
    fn safe_relative(&self, value: &str) -> Result<(String, PathBuf)> {
        let head = value;
        if Path::new(head).is_absolute()
            || head.contains('\\')
            || head.contains('\0')
            || head.contains(':')
            || head.split('/').any(|part| part == "..")
        {
            return Err(RuntimeError::policy(
                "Handoff file paths must be project-relative",
            ));
        }
        let relative = head
            .split('/')
            .filter(|part| !part.is_empty() && *part != ".")
            .collect::<Vec<_>>()
            .join("/");
        if relative.is_empty() {
            return Err(RuntimeError::invalid("Empty handoff file path"));
        }
        for component in relative.split('/') {
            let lower = component.to_lowercase();
            if matches!(
                lower.as_str(),
                ".git" | "secret" | "secrets" | "credentials"
            ) || lower.starts_with(".env")
                || lower.starts_with("id_")
                || ["pem", "key", "p12", "pfx", "jks", "keystore"]
                    .contains(&lower.rsplit('.').next().unwrap_or(""))
            {
                return Err(RuntimeError::policy("Sensitive handoff file path"));
            }
        }
        let path = self.context.roots.project.join(&relative);
        fs::admit_descendant(&self.context.roots.project, &path)?;
        Ok((relative, path))
    }
    pub fn fingerprint_files(&self, values: &[String]) -> Result<Vec<Value>> {
        if values.len() > 256 {
            return Err(RuntimeError::invalid(
                "At most 256 source files may be referenced",
            ));
        }
        let frame = git::GitFrame::discover(&self.context.roots.project)?;
        let mut seen = BTreeSet::new();
        let mut rows = Vec::new();
        for value in values {
            let (relative, path) = self.safe_relative(value)?;
            if !seen.insert(relative.clone()) {
                return Err(RuntimeError::invalid("Handoff file paths must be unique"));
            }
            if let Some(frame) = &frame
                && !frame.ignored(std::slice::from_ref(&relative))?.is_empty()
            {
                return Err(RuntimeError::policy(
                    "Ignored handoff files cannot be included",
                ));
            }
            let checksum = match std::fs::File::open(&path) {
                Ok(mut handle) => {
                    if !handle.metadata()?.is_file() {
                        return Err(RuntimeError::policy("Handoff source is not a regular file"));
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
            return Err(RuntimeError::invalid("Invalid source client"));
        }
        if (input.detail == "topic")
            != input
                .topic
                .as_ref()
                .is_some_and(|topic| !results::clean(topic).is_empty())
            || (input.detail == "full") != input.transcript.is_some()
        {
            return Err(RuntimeError::invalid(
                "Topic/transcript fields do not match handoff detail",
            ));
        }
        domain::guard_shared_text(
            "topic and task ID",
            &input
                .topic
                .iter()
                .chain(input.task_id.iter())
                .cloned()
                .collect::<Vec<_>>(),
        )?;
        let context = self.validate_curated(&input.input, &input.detail)?;
        let files = values(&context["files"])?;
        let fingerprints = self.fingerprint_files(&files)?;
        no_link_parents(&input.output)?;
        let output = fs::absolute(&input.output)?;
        if output.extension().and_then(|value| value.to_str()) != Some("md") {
            return Err(RuntimeError::invalid(
                "Handoff output must use a .md extension",
            ));
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
                "Handoff must be a standalone task artifact",
            ));
        }
        if self.context.roots.attached() && output.starts_with(&self.context.roots.state) {
            return Err(RuntimeError::policy(
                "Handoff must lie outside accelerator state",
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
                "Encoded handoff exceeds its size limit",
            ));
        }
        let metadata = object!({"schema_version":if encoded{2}else{1},"type":"context-handoff","created_at":clock::now().to_rfc3339(),"detail":input.detail,"topic":input.topic.map(|value|results::clean(&value)),"source_client":input.source_client,"task_id":input.task_id.map(|value|results::clean(&value)).filter(|value|!value.is_empty()),"repository":{"branch":self.git_value(&["branch","--show-current"]),"commit":self.git_value(&["rev-parse","HEAD"])},"files":fingerprints,"context":context,"body_sha256":result_store::sha(&body),"transcript":transcript.as_ref().map(|raw|object!({"sha256":result_store::sha(raw),"bytes":raw.len()}))});
        if domain::secret_label(&format!(
            "{}{}",
            json::python_default(&metadata, false, false, true)?,
            String::from_utf8_lossy(&body)
        ))?
        .is_some()
        {
            return Err(RuntimeError::policy("Handoff contains a credential"));
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
        let mut handle = options.open(&output)?;
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
    fn decode(&self, raw: &[u8]) -> Result<(Value, Value, Option<Vec<u8>>)> {
        let text =
            std::str::from_utf8(raw).map_err(|_| RuntimeError::invalid("Handoff is not UTF-8"))?;
        let split = memory_kernel_probe::split_frontmatter(text)
            .map_err(|_| RuntimeError::invalid("Handoff is missing JSON frontmatter"))?;
        let metadata = json::parse(split.metadata_text)?;
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
        if !result_store::exact(&metadata, &required)
            || !matches!(metadata["schema_version"].as_u64(), Some(1 | 2))
            || metadata["type"] != "context-handoff"
            || !metadata["created_at"]
                .as_str()
                .is_some_and(domain::valid_datetime)
            || !matches!(
                metadata["source_client"].as_str(),
                Some("codex" | "claude" | "cursor" | "other")
            )
            || (!metadata["task_id"].is_null() && !metadata["task_id"].is_string())
            || !result_store::exact(&metadata["repository"], &["branch", "commit"])
        {
            return Err(RuntimeError::invalid("Unsupported handoff metadata schema"));
        }
        let detail = metadata["detail"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("Invalid handoff detail"))?;
        if (detail == "topic")
            != metadata["topic"]
                .as_str()
                .is_some_and(|topic| !results::clean(topic).is_empty())
            || detail != "topic" && !metadata["topic"].is_null()
        {
            return Err(RuntimeError::invalid("Invalid handoff topic"));
        }
        let context = self.validate_curated(&metadata["context"], detail)?;
        let fingerprints = metadata["files"]
            .as_array()
            .ok_or_else(|| RuntimeError::invalid("Invalid handoff file image"))?;
        let file_names = values(&context["files"])?;
        if fingerprints.len() != file_names.len() {
            return Err(RuntimeError::invalid(
                "Handoff fingerprints do not match file references",
            ));
        }
        for (row, name) in fingerprints.iter().zip(&file_names) {
            let (relative, _) = self.safe_relative(name)?;
            if !result_store::exact(row, &["path", "sha256"])
                || row["path"] != relative
                || !row["sha256"].is_null()
                    && !result_store::hex(row["sha256"].as_str().unwrap_or(""), 64)
            {
                return Err(RuntimeError::invalid("Invalid handoff fingerprint"));
            }
        }
        let mut body = render_body(&context)?.into_bytes();
        let transcript = if metadata["transcript"].is_null() {
            if detail == "full" {
                return Err(RuntimeError::invalid("Full handoff requires a transcript"));
            }
            None
        } else {
            if detail != "full"
                || !result_store::exact(&metadata["transcript"], &["sha256", "bytes"])
            {
                return Err(RuntimeError::invalid(
                    "Only a full handoff may contain a transcript",
                ));
            }
            let encoded = metadata["schema_version"] == 2;
            body.extend_from_slice(if encoded {
                ENCODED_MARKER.as_bytes()
            } else {
                MARKER.as_bytes()
            });
            let stored = split
                .body
                .as_bytes()
                .strip_prefix(body.as_slice())
                .ok_or_else(|| RuntimeError::invalid("Handoff transcript marker is missing"))?;
            let raw = if encoded {
                let decoded: Value = json::parse(
                    std::str::from_utf8(stored)
                        .map_err(|_| RuntimeError::invalid("Invalid transcript encoding"))?,
                )?;
                let text = decoded
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("Invalid transcript encoding"))?;
                if domain::secret_label(text)?.is_some() {
                    return Err(RuntimeError::policy("Transcript contains a credential"));
                }
                body.extend_from_slice(
                    json::python_default(&decoded, false, false, true)?.as_bytes(),
                );
                text.as_bytes().to_vec()
            } else {
                body.extend_from_slice(stored);
                stored.to_vec()
            };
            if raw.is_empty()
                || raw.len() > TRANSCRIPT_LIMIT
                || metadata["transcript"]["bytes"].as_u64() != Some(raw.len() as u64)
                || metadata["transcript"]["sha256"] != result_store::sha(&raw)
            {
                return Err(RuntimeError::invalid(
                    "Handoff transcript digest does not match",
                ));
            }
            Some(raw)
        };
        if body != split.body.as_bytes() || metadata["body_sha256"] != result_store::sha(&body) {
            return Err(RuntimeError::invalid(
                "Handoff body does not match curated metadata",
            ));
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
