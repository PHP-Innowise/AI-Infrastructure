//! The executor's state file.
//!
//! One file per run, replaced atomically and flushed to disk before every
//! program starts, so a crashed or timed-out command resumes from what was
//! decided rather than from what was remembered. Next to it, `run-log.jsonl`
//! names each phase of the run for the Harness; it carries no values.
use infra_core::{Result, RuntimeError};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

pub const STATE_SCHEMA: u64 = 1;
const STATE_BYTES: usize = 8 * 1024 * 1024;
pub const LOG_FILE: &str = "run-log.jsonl";

/// What the run waits for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Awaiting {
    /// A program is about to start or is running. Written before it starts.
    Running { step_index: usize, attempt: u8 },
    /// A program needs values that are not bound yet.
    Bindings {
        step_index: usize,
        names: Vec<String>,
    },
    /// The agent performs these instruction or inspection steps.
    Steps {
        step_indexes: Vec<usize>,
        produces: Vec<String>,
    },
    /// A program failed; the agent supplies the outputs later programs read.
    Outputs {
        step_index: usize,
        names: Vec<String>,
    },
    /// Nothing: the plan is finished, or has not been advanced yet.
    None,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Failure {
    pub step_index: usize,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunState {
    pub capsule_sha256: String,
    pub generation: String,
    pub position: usize,
    pub awaiting: Awaiting,
    pub bindings: Map<String, Value>,
    pub failed: Vec<Failure>,
    pub completed: Vec<usize>,
    /// The result of each completed program, by program identifier.
    pub results: Map<String, Value>,
}

fn names(value: &Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|name| name.as_str().map(String::from))
        .collect()
}
fn index(value: &Value) -> Option<usize> {
    value.as_u64().and_then(|index| usize::try_from(index).ok())
}

impl RunState {
    pub fn new(capsule_sha256: &str, generation: &str) -> Self {
        Self {
            capsule_sha256: capsule_sha256.into(),
            generation: generation.into(),
            position: 0,
            awaiting: Awaiting::None,
            bindings: Map::new(),
            failed: Vec::new(),
            completed: Vec::new(),
            results: Map::new(),
        }
    }
    pub fn is_failed(&self, step_index: usize) -> bool {
        self.failed
            .iter()
            .any(|failure| failure.step_index == step_index)
    }
    pub fn to_value(&self) -> Value {
        let awaiting = match &self.awaiting {
            Awaiting::Running {
                step_index,
                attempt,
            } => json!({"kind": "running", "step_index": step_index, "attempt": attempt}),
            Awaiting::Bindings { step_index, names } => {
                json!({"kind": "bindings", "step_index": step_index, "names": names})
            }
            Awaiting::Steps {
                step_indexes,
                produces,
            } => json!({"kind": "steps", "step_indexes": step_indexes, "produces": produces}),
            Awaiting::Outputs { step_index, names } => {
                json!({"kind": "outputs", "step_index": step_index, "names": names})
            }
            Awaiting::None => json!({"kind": "none"}),
        };
        json!({
            "schema_version": STATE_SCHEMA,
            "capsule_sha256": self.capsule_sha256,
            "generation": self.generation,
            "position": self.position,
            "awaiting": awaiting,
            "bindings": self.bindings,
            "failed": self.failed.iter().map(|failure| json!({
                "step_index": failure.step_index, "reason": failure.reason,
            })).collect::<Vec<_>>(),
            "completed": self.completed,
            "results": self.results,
        })
    }
    pub fn from_value(value: &Value) -> Option<Self> {
        if value["schema_version"].as_u64() != Some(STATE_SCHEMA) {
            return None;
        }
        let waiting = &value["awaiting"];
        let awaiting = match waiting["kind"].as_str()? {
            "running" => Awaiting::Running {
                step_index: index(&waiting["step_index"])?,
                attempt: u8::try_from(waiting["attempt"].as_u64()?).ok()?,
            },
            "bindings" => Awaiting::Bindings {
                step_index: index(&waiting["step_index"])?,
                names: names(&waiting["names"])?,
            },
            "steps" => Awaiting::Steps {
                step_indexes: waiting["step_indexes"]
                    .as_array()?
                    .iter()
                    .map(index)
                    .collect::<Option<_>>()?,
                produces: names(&waiting["produces"])?,
            },
            "outputs" => Awaiting::Outputs {
                step_index: index(&waiting["step_index"])?,
                names: names(&waiting["names"])?,
            },
            "none" => Awaiting::None,
            _ => return None,
        };
        Some(Self {
            capsule_sha256: value["capsule_sha256"].as_str()?.into(),
            generation: value["generation"].as_str()?.into(),
            position: index(&value["position"])?,
            awaiting,
            bindings: value["bindings"].as_object()?.clone(),
            failed: value["failed"]
                .as_array()?
                .iter()
                .map(|failure| {
                    Some(Failure {
                        step_index: index(&failure["step_index"])?,
                        reason: failure["reason"].as_str()?.into(),
                    })
                })
                .collect::<Option<_>>()?,
            completed: value["completed"]
                .as_array()?
                .iter()
                .map(index)
                .collect::<Option<_>>()?,
            results: value["results"].as_object()?.clone(),
        })
    }
    /// Reads the state file; `None` when no run has started.
    pub fn load(path: &Path) -> Result<Option<Self>> {
        let bytes = match infra_core::fs::read_regular(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind == infra_core::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let value = infra_core::strict_json::parse(&bytes, STATE_BYTES, 32, 200_000)?;
        Self::from_value(&value)
            .map(Some)
            .ok_or_else(|| RuntimeError::invalid("The executor state file is not readable."))
    }
    /// Replaces the state file and flushes it to disk before returning.
    pub fn save(&self, path: &Path) -> Result<()> {
        let mut bytes = serde_json::to_vec(&self.to_value())?;
        bytes.push(b'\n');
        infra_core::fs::atomic_write(path, &bytes)
    }
}

/// The append-only journal of a run's phases, beside its state file.
pub struct RunLog {
    path: PathBuf,
    generation: String,
    seq: u64,
    needs_newline: bool,
}
impl RunLog {
    /// Opens the journal that belongs to `state_file`, continuing its numbering.
    pub fn open(state_file: &Path, generation: &str) -> Result<Self> {
        Self::open_log(log_path(state_file), generation)
    }
    /// Opens a journal at `path`, continuing its numbering from the highest
    /// valid line and separating any torn trailing partial line before writing.
    pub fn open_log(path: PathBuf, generation: &str) -> Result<Self> {
        let (seq, needs_newline) = match infra_core::fs::read_regular(&path) {
            Ok(bytes) => {
                let mut seq = 0u64;
                for line in bytes
                    .split(|byte| *byte == b'\n')
                    .filter(|line| !line.is_empty())
                {
                    if let Ok(event) = infra_core::strict_json::parse(line, 64 * 1024, 8, 1000)
                        && let Some(number) = event["seq"].as_u64()
                    {
                        seq = seq.max(number);
                    }
                }
                let needs_newline = !bytes.is_empty() && !bytes.ends_with(b"\n");
                (seq, needs_newline)
            }
            Err(error) if error.kind == infra_core::ErrorKind::NotFound => (0, false),
            Err(error) => return Err(error),
        };
        Ok(Self {
            path,
            generation: generation.into(),
            seq,
            needs_newline,
        })
    }
    /// Appends one event and flushes it. `detail` names steps and programs,
    /// never a bound value or a result.
    pub fn append(&mut self, event: &str, detail: Value) -> Result<()> {
        let next = self.seq + 1;
        let mut line = Map::new();
        line.insert("seq".into(), next.into());
        line.insert("generation".into(), self.generation.clone().into());
        line.insert(
            "at_ms".into(),
            (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |after| after.as_millis() as u64))
            .into(),
        );
        line.insert("event".into(), event.into());
        if let Value::Object(detail) = detail {
            line.extend(detail);
        }
        let mut bytes = Vec::new();
        if self.needs_newline {
            bytes.push(b'\n');
        }
        serde_json::to_writer(&mut bytes, &Value::Object(line))?;
        bytes.push(b'\n');
        infra_core::fs::append_whole_line(&self.path, &bytes)?;
        self.seq = next;
        self.needs_newline = false;
        Ok(())
    }
}
pub fn log_path(state_file: &Path) -> PathBuf {
    state_file.with_file_name(LOG_FILE)
}

/// The journal's events after `after_seq`, in order. A torn or malformed line,
/// which a crash can leave, is skipped without failing or hiding later lines.
pub fn read_log_at(log_file: &Path, after_seq: u64) -> Result<Vec<Value>> {
    let bytes = match infra_core::fs::read_regular(log_file) {
        Ok(bytes) => bytes,
        Err(error) if error.kind == infra_core::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut events = Vec::new();
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let Ok(event) = infra_core::strict_json::parse(line, 64 * 1024, 8, 1000) else {
            continue;
        };
        if event["seq"].as_u64().is_some_and(|seq| seq > after_seq) {
            events.push(event);
        }
    }
    Ok(events)
}

/// The journal's events after `after_seq` beside `state_file`.
pub fn read_log(state_file: &Path, after_seq: u64) -> Result<Vec<Value>> {
    read_log_at(&log_path(state_file), after_seq)
}
