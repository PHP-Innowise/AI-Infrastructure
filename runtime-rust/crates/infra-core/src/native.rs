//! Native prompt/Stop/result lifecycle and bounded sandbox spool.
use crate::{
    ErrorKind, Result, RuntimeConfig, RuntimeContext, RuntimeError,
    domain::{clock, json, uuid4},
    fs,
    result_store::{self, EvidenceStore, PROOF_PROFILE},
    results::{self, RecordResult, ResultService, Revision},
    serving::TaskResolver,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json as object};
use std::{
    collections::BTreeSet,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
pub const PROFILE: &str = "native-result-challenge-v1";
pub const REASON: &str = "Automatic native result; agent-attested, not reviewed by a person";
pub const MAX_INPUT: usize = 1024 * 1024;
pub const MAX_STATES: usize = 128;
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Host {
    Claude,
    Codex,
    Cursor,
}
impl Host {
    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Cursor => "cursor",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeState {
    pub profile: String,
    pub owner: String,
    pub host: Host,
    pub session: String,
    pub project: String,
    pub task_id: String,
    pub token: String,
    pub phase: String,
    pub task_uuid: Option<String>,
    pub revision: Option<u64>,
    pub scope: Option<String>,
    pub digest: Option<String>,
    pub updated: f64,
    pub checkout: Option<String>,
    pub adapter: Option<String>,
}
#[derive(Clone, Debug)]
pub struct NativeStop {
    pub task_id: String,
    pub host: Host,
    pub payload: Value,
    pub entered: bool,
}
#[derive(Clone, Debug)]
pub struct NativeSave {
    pub key: String,
    pub token: String,
    pub data: Value,
    pub skip: bool,
    pub spooled: bool,
}
#[derive(Clone, Debug)]
pub struct NativeDrain {
    pub task_id: String,
    pub host: Host,
    pub payload: Value,
}
#[derive(Clone)]
pub struct NativeService {
    pub context: RuntimeContext,
    pub results: ResultService,
}
pub fn error(code: &str) -> RuntimeError {
    let code = format!("native-result:{code}");
    RuntimeError::coded(ErrorKind::Policy, &code, &code)
}
pub fn seconds() -> f64 {
    clock::proof_now().timestamp_micros() as f64 / 1_000_000.0
}
pub fn adapter() -> String {
    result_store::sha(concat!(include_str!("native.rs"), include_str!("review.rs")).as_bytes())
}
pub fn parse_input(raw: &[u8]) -> Result<Value> {
    if raw.len() > MAX_INPUT {
        return Err(error("input-too-large"));
    }
    let value = crate::strict_json::parse(raw, MAX_INPUT, 512, MAX_INPUT)
        .map_err(|_| error("invalid-input"))?;
    if !value.is_object() {
        return Err(error("invalid-input"));
    }
    Ok(value)
}
impl NativeService {
    pub fn new(context: RuntimeContext) -> Self {
        Self {
            results: ResultService::new(context.clone()),
            context,
        }
    }
    pub fn read_input(&self, raw: &[u8], key: Option<&str>) -> Result<Value> {
        let value = parse_input(raw);
        if let Err(error) = &value {
            let host = key
                .and_then(|key| self.read_state(key, None).ok().flatten())
                .map(|state| state.host);
            self.journal(
                host,
                "save",
                "refused",
                Some(error.refusal_code()),
                &Value::Null,
            );
        }
        value
    }
    pub fn project(&self) -> String {
        result_store::sha(self.context.roots.project.to_string_lossy().as_bytes())
    }
    pub fn enabled(&self) -> bool {
        self.context.mode == "governed"
            && !["CONTEXT_MEMORY_RECOVERY", "CONTEXT_CAPSULE_DELIVERED"]
                .iter()
                .any(|key| std::env::var(key).as_deref() == Ok("1"))
    }
    pub fn setting(
        &self,
        environment: &str,
        key: &str,
        default: &str,
        allowed: &[&str],
    ) -> Result<String> {
        let config = RuntimeConfig::load(&self.context.roots)?;
        let value = std::env::var(environment)
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(|| config.value[key].as_str().map(String::from))
            .unwrap_or_else(|| default.into());
        if !allowed.contains(&value.as_str()) {
            return Err(error(&format!(
                "invalid-{}-setting",
                key.strip_prefix("native_result_").unwrap_or(key)
            )));
        }
        Ok(value)
    }
    pub fn key(&self, task: &str, host: Host, payload: &Value) -> Result<Option<String>> {
        let field = if host == Host::Cursor {
            "conversation_id"
        } else {
            "session_id"
        };
        let Some(session) = payload[field].as_str() else {
            return Ok(None);
        };
        if session.is_empty()
            || session.len() > 160
            || !session
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
        {
            return Ok(None);
        }
        Ok(Some(result_store::legacy_digest(
            &object!([
                self.project(),
                self.context.actor,
                host.name(),
                result_store::sha(session.as_bytes()),
                task
            ]),
            false,
            true,
        )?))
    }
    pub fn state_path(&self, key: &str) -> Result<PathBuf> {
        if !result_store::hex(key, 64) {
            return Err(error("invalid-request"));
        }
        let path = self
            .context
            .roots
            .brain_root()
            .join("local/native-results")
            .join(format!("{key}.json"));
        fs::admit_descendant(&self.context.roots.state, &path)?;
        Ok(path)
    }
    pub fn read_state(&self, key: &str, slot: Option<&str>) -> Result<Option<NativeState>> {
        let path = self.state_path(slot.unwrap_or(key))?;
        let Some(raw) =
            result_store::read_bounded(&path, 4096).map_err(|_| error("invalid-state"))?
        else {
            return Ok(None);
        };
        let state: NativeState = serde_json::from_value(
            crate::strict_json::parse(&raw, 4096, 512, 4096).map_err(|_| error("invalid-state"))?,
        )
        .map_err(|_| error("invalid-state"))?;
        if state.profile != PROFILE
            || !matches!(
                state.phase.as_str(),
                "armed"
                    | "requested"
                    | "saved"
                    | "held"
                    | "omitted"
                    | "failed"
                    | "refused"
                    | "declined"
            )
            || !result_store::hex(&state.token, 32)
            || !state.updated.is_finite()
            || state.updated < 0.0
            || state.revision == Some(0)
            || !result_store::hex(&state.session, 64)
            || !result_store::hex(&state.project, 64)
            || [
                state.scope.as_ref(),
                state.digest.as_ref(),
                state.checkout.as_ref(),
                state.adapter.as_ref(),
            ]
            .iter()
            .flatten()
            .any(|value| !result_store::hex(value, 64))
            || state
                .task_uuid
                .as_deref()
                .is_some_and(|id| !crate::domain::is_uuid4(id))
        {
            return Err(error("invalid-state"));
        }
        results::text(&object!(state.owner), "owner", 200)?;
        results::task_id(&state.task_id)?;
        let expected = result_store::legacy_digest(
            &object!([
                state.project,
                state.owner,
                state.host.name(),
                state.session,
                state.task_id
            ]),
            false,
            true,
        )?;
        if expected != key || state.project != self.project() {
            return Err(error("scope-changed"));
        }
        Ok(Some(state))
    }
    fn write_state(&self, slot: &str, state: &NativeState) -> Result<()> {
        fs::atomic_json(&self.state_path(slot)?, &serde_json::to_value(state)?)
    }
    fn armed(&self, task: &str, host: Host, payload: &Value) -> NativeState {
        let session = payload[if host == Host::Cursor {
            "conversation_id"
        } else {
            "session_id"
        }]
        .as_str()
        .unwrap_or("");
        NativeState {
            profile: PROFILE.into(),
            owner: self.context.actor.clone(),
            host,
            session: result_store::sha(session.as_bytes()),
            project: self.project(),
            task_id: task.into(),
            token: uuid4().replace('-', ""),
            phase: "armed".into(),
            task_uuid: None,
            revision: None,
            scope: None,
            digest: None,
            checkout: None,
            adapter: None,
            updated: seconds(),
        }
    }
    fn aside(key: &str) -> String {
        result_store::sha(format!("aside:{key}").as_bytes())
    }
    pub fn journal(
        &self,
        host: Option<Host>,
        stage: &str,
        event: &str,
        code: Option<&str>,
        counts: &Value,
    ) {
        let result = (|| -> Result<()> {
            let mut row = object!({"at":clock::proof_now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),"host":host.map(Host::name),"stage":stage,"event":event});
            if let Some(code) = code {
                row["code"] = code.into();
            }
            if let Some(counts) = counts.as_object() {
                for (key, value) in counts {
                    if [
                        "learnings",
                        "submitted",
                        "existing",
                        "held",
                        "promoted",
                        "capture_decision",
                        "answer",
                    ]
                    .contains(&key.as_str())
                    {
                        row[key] = value.clone();
                    }
                }
            }
            let path = self
                .context
                .roots
                .state
                .join("memory-bank/local/native-results.ndjson");
            let mut raw = json::canonical(&row)?.into_bytes();
            raw.push(b'\n');
            fs::append_whole_line(&path, &raw)?;
            let config = RuntimeConfig::load(&self.context.roots)?;
            let keep = config.value["refresh_health_retention"]
                .as_u64()
                .unwrap_or(500)
                .max(1) as usize
                * 8;
            if path.metadata()?.len() > keep as u64 * 256
                && let Some(raw) = result_store::read_bounded(&path, 64 * 1024 * 1024)?
            {
                let lines = raw
                    .split_inclusive(|byte| *byte == b'\n')
                    .collect::<Vec<_>>();
                if lines.len() > keep {
                    fs::atomic_write(&path, &lines[lines.len() - keep..].concat())?;
                }
            }
            Ok(())
        })();
        let _ = result;
    }
    /// Why `enabled` is false, as the code a journal line keeps.
    fn disabled(&self) -> &'static str {
        if self.context.mode != "governed" {
            "not-governed"
        } else if std::env::var("CONTEXT_CAPSULE_DELIVERED").as_deref() == Ok("1") {
            "capsule-delivered"
        } else {
            "recovery-session"
        }
    }
    fn observed(
        &self,
        stage: &str,
        host: Option<Host>,
        mut result: Result<Value>,
    ) -> Result<Value> {
        match &mut result {
            Ok(value) => {
                // The journal's event and code: the funnel report, the host
                // benchmark and troubleshooting read why a Stop asked nothing
                // or how a continuation settled the request.
                let noted = value
                    .as_object_mut()
                    .and_then(|map| map.remove("_journal"))
                    .unwrap_or(Value::Null);
                let event = if let Some(event) = noted["event"].as_str() {
                    event
                } else if value["armed"] == true {
                    "armed"
                } else if value["continuation"] == true {
                    "continuation"
                } else if let Some(state) = value["state"].as_str() {
                    state
                } else if value.get("decision").is_some()
                    || value.get("followup_message").is_some()
                    || value.get("hookSpecificOutput").is_some()
                {
                    "requested"
                } else {
                    "skipped"
                };
                self.journal(host, stage, event, noted["code"].as_str(), &noted);
            }
            Err(error) => self.journal(
                host,
                stage,
                if stage == "save" { "refused" } else { "error" },
                Some(error.refusal_code()),
                &Value::Null,
            ),
        }
        result
    }
    pub fn begin(&self, task: &str, host: Host, payload: &Value) -> Result<Value> {
        let outcome = (|| {
            let task = results::task_id(task)?;
            if !self.enabled() {
                return Ok(noted(object!({"armed":false}), "skipped", self.disabled()));
            }
            if self.setting(
                "CONTEXT_NATIVE_RESULT_REQUEST",
                "native_result_request",
                "every-turn",
                &["every-turn", "off"],
            )? == "off"
            {
                return Ok(noted(object!({"armed":false}), "skipped", "switched-off"));
            }
            let Some(key) = self.key(&task, host, payload)? else {
                return Ok(noted(object!({"armed":false}), "skipped", "no-session-id"));
            };
            let brain = self.results.brain()?;
            let _guard = brain.lock()?;
            // A state this runtime cannot read - another version wrote it, or
            // it is damaged - is replaced by the new request.
            let mut unreadable = false;
            let mut previous = match self.read_state(&key, None) {
                Ok(state) => state,
                Err(error) if error.refusal_code() == "native-result:invalid-state" => {
                    unreadable = true;
                    None
                }
                Err(error) => return Err(error),
            };
            let aside_key = Self::aside(&key);
            let aside = self.read_state(&key, Some(&aside_key)).ok().flatten();
            for state in previous.iter().chain(aside.iter()) {
                if state.phase == "requested"
                    && payload["prompt"].as_str().is_some_and(|prompt| {
                        prompt
                            .trim_start()
                            .starts_with(&format!("INTERNAL_NATIVE_MEMORY_RESULT {}", state.token))
                    })
                {
                    return Ok(object!({"armed":false,"continuation":true}));
                }
            }
            let mut unsettled = None::<String>;
            if let Some(mut old) = aside {
                let _ = std::fs::remove_file(self.state_path(&aside_key)?);
                if matches!(old.phase.as_str(), "requested" | "failed")
                    && host == Host::Claude
                    && old.digest.is_none()
                    && claude_refusal(payload, &old.token)
                {
                    old.phase = "refused".into();
                    old.updated = seconds();
                }
                if old.phase == "refused"
                    && previous.as_ref().is_none_or(|state| state.phase == "armed")
                {
                    previous = Some(old);
                } else if matches!(
                    old.phase.as_str(),
                    "requested" | "failed" | "held" | "refused"
                ) {
                    unsettled = Some(old.phase);
                }
            }
            if let Some(state) = previous.as_mut() {
                if matches!(state.phase.as_str(), "requested" | "failed")
                    && host == Host::Claude
                    && state.digest.is_none()
                    && claude_refusal(payload, &state.token)
                {
                    state.phase = "refused".into();
                    state.updated = seconds();
                }
                let age = seconds() - state.updated;
                if matches!(state.phase.as_str(), "refused" | "declined")
                    && (0.0..86400.0).contains(&age)
                {
                    if state.phase == "declined" {
                        return Ok(noted(object!({"armed":false}), "skipped", "host-refused"));
                    }
                    state.phase = "declined".into();
                    state.updated = seconds();
                    self.write_state(&key, state)?;
                    return Ok(noted(
                        object!({"armed":false,"warning":"Previous native memory result was not saved completely."}),
                        "skipped",
                        "host-refused",
                    ));
                }
            }
            let moved = previous.as_ref().is_some_and(|state| {
                state.phase == "requested" && (0.0..600.0).contains(&(seconds() - state.updated))
            });
            if moved {
                self.write_state(
                    &aside_key,
                    previous.as_ref().ok_or_else(|| error("invalid-state"))?,
                )?;
            }
            let warning = unsettled.is_some()
                || !moved
                    && previous.as_ref().is_some_and(|state| {
                        matches!(
                            state.phase.as_str(),
                            "requested" | "failed" | "held" | "refused"
                        )
                    });
            self.write_state(&key, &self.armed(&task, host, payload))?;
            self.prune_states(&key)?;
            let mut output = object!({"armed":true,"request":key});
            if warning {
                output["warning"] =
                    "Previous native memory result was not saved completely.".into();
            }
            // The warning is counted by what the last request became.
            let code = if warning {
                unsettled
                    .or_else(|| previous.as_ref().map(|state| state.phase.clone()))
                    .map(|phase| format!("previous-{phase}"))
            } else if moved {
                Some("previous-in-flight".into())
            } else if unreadable {
                Some("previous-unreadable".into())
            } else {
                None
            };
            Ok(match code {
                Some(code) => noted(output, "armed", &code),
                None => output,
            })
        })();
        self.observed("prompt", Some(host), outcome)
    }
    pub fn proven(&self, state: &NativeState) -> Result<bool> {
        let (Some(task), Some(digest)) = (&state.task_uuid, &state.digest) else {
            return Ok(false);
        };
        let proof = EvidenceStore::new(self.context.clone()).attempt(
            task,
            &format!("native-{}", state.token),
            digest,
        )?;
        Ok(proof.is_some_and(|proof| {
            proof.profile == PROOF_PROFILE
                && proof.state == "complete"
                && proof.owner == state.owner
                && proof.attestation == "agent"
        }))
    }
    pub fn stop(&self, input: NativeStop, tasks: &mut impl TaskResolver) -> Result<Value> {
        let host = input.host;
        let outcome = (|| {
            if !self.enabled() {
                return Ok(noted(object!({}), "skipped", self.disabled()));
            }
            if self.setting(
                "CONTEXT_NATIVE_RESULT_REQUEST",
                "native_result_request",
                "every-turn",
                &["every-turn", "off"],
            )? == "off"
            {
                return Ok(noted(object!({}), "skipped", "switched-off"));
            }
            let task = results::task_id(&input.task_id)?;
            let Some(key) = self.key(&task, host, &input.payload)? else {
                return Ok(noted(object!({}), "skipped", "no-session-id"));
            };
            if host == Host::Cursor && input.payload["status"] != "completed" {
                return Ok(noted(object!({}), "skipped", "not-completed"));
            }
            if host != Host::Cursor
                && input
                    .payload
                    .get("hook_event_name")
                    .is_some_and(|value| value != "Stop")
            {
                return Ok(noted(object!({}), "skipped", "not-stop-event"));
            }
            let brain = self.results.brain()?;
            let _guard = brain.lock()?;
            let recovery = recovery(host, &input.payload);
            let mut state = self.read_state(&key, None)?;
            if state.is_none() && input.entered && !recovery {
                state = Some(self.armed(&task, host, &input.payload));
                self.write_state(&key, state.as_ref().ok_or_else(|| error("invalid-state"))?)?;
            }
            let Some(mut state) = state else {
                return Ok(noted(object!({}), "skipped", "not-armed"));
            };
            if recovery {
                let mut slot = key.clone();
                if state.phase != "requested" {
                    let aside = Self::aside(&key);
                    if let Some(other) = self.read_state(&key, Some(&aside)).ok().flatten() {
                        state = other;
                        slot = aside;
                    }
                }
                if state.phase == "requested" {
                    state.phase = if self.proven(&state)? {
                        "held"
                    } else if host == Host::Claude
                        && state.digest.is_none()
                        && claude_refusal(&input.payload, &state.token)
                    {
                        "refused"
                    } else {
                        "failed"
                    }
                    .into();
                    self.write_state(&slot, &state)?;
                    // Held: a finished save the state never took. Failed: no
                    // save began, or one began and did not finish. Refused:
                    // Claude Code refused the save command itself.
                    let code = match state.phase.as_str() {
                        "held" => "unconfirmed",
                        "refused" => "host-refused",
                        _ if state.digest.is_some() => "incomplete",
                        _ => "unsaved",
                    };
                    let event = if state.phase == "held" {
                        "held"
                    } else {
                        "failed"
                    };
                    return Ok(noted(object!({}), event, code));
                }
                return Ok(noted(
                    object!({}),
                    "skipped",
                    &format!("recovery-{}", state.phase),
                ));
            }
            if state.phase != "armed" {
                return Ok(noted(
                    object!({}),
                    "skipped",
                    &format!("phase-{}", state.phase),
                ));
            }
            let turn = if host == Host::Codex {
                rollout_turn(&input.payload)
            } else {
                Value::Null
            };
            if input.payload["permission_mode"] == "plan"
                || turn["collaboration_mode"]["mode"] == "plan"
            {
                return Ok(noted(object!({}), "skipped", "plan-mode"));
            }
            if turn["sandbox_policy"]["type"] == "read-only" {
                return Ok(noted(object!({}), "skipped", "read-only-sandbox"));
            }
            let policy = &turn["file_system_sandbox_policy"];
            if policy["kind"] == "restricted"
                && turn["cwd"]
                    .as_str()
                    .is_some_and(|cwd| sandbox_writes(policy, Path::new(cwd)))
                && !sandbox_writes(policy, &self.context.roots.brain_root())
                && !spool_directories().iter().any(|folder| {
                    folder
                        .parent()
                        .is_some_and(|parent| sandbox_writes(policy, parent))
                })
            {
                return Ok(noted(object!({}), "skipped", "outside-sandbox"));
            }
            let current = match self.task(&task, tasks) {
                Ok(task) => Some(task),
                Err(error) if error.kind == ErrorKind::NotFound => None,
                Err(error) if error.kind == ErrorKind::InvalidInput => {
                    return Ok(noted(object!({}), "skipped", "task-unreadable"));
                }
                Err(error) => return Err(error),
            };
            if let Some(task) = current {
                if let Some(status @ ("completed" | "cancelled")) = task["status"].as_str() {
                    return Ok(noted(object!({}), "skipped", &format!("task-{status}")));
                }
                state.task_uuid = task["id"].as_str().map(String::from);
                state.revision = task["revision"].as_u64();
            }
            state.phase = "requested".into();
            state.scope = Some(self.results.scope(REASON, "agent")?);
            state.checkout = Some(checkout(&self.context)?);
            state.adapter = Some(adapter());
            state.updated = seconds();
            self.write_state(&key, &state)?;
            let (instruction, brief) =
                self.instruction(&key, &state, &input.payload, input.entered)?;
            let mut output = match host {
                Host::Claude => {
                    object!({"hookSpecificOutput":{"hookEventName":"Stop","additionalContext":instruction}})
                }
                Host::Cursor => object!({"followup_message":instruction}),
                Host::Codex => object!({"decision":"block","reason":instruction}),
            };
            output["_journal"] =
                object!({"event":"requested","answer":if brief {"brief"} else {"repeat"}});
            Ok(output)
        })();
        self.observed("stop", Some(host), outcome)
    }
    fn task(&self, id: &str, tasks: &impl TaskResolver) -> Result<Value> {
        match self.results.task(id, false) {
            Ok(image) => Ok(image.record),
            Err(failure) if failure.kind == ErrorKind::Conflict => {
                let uuid = tasks.served_uuid(id)?.ok_or(failure)?;
                let task = self.results.task(&uuid, false)?.record;
                if task["external_id"] != id {
                    return Err(error("task-changed"));
                }
                Ok(task)
            }
            Err(failure) => Err(failure),
        }
    }
    fn check_scope(&self, state: &NativeState) -> Result<()> {
        if state.owner != self.context.actor || state.project != self.project() {
            return Err(error("request-unavailable"));
        }
        if seconds() - state.updated > 86400.0
            || state.updated > seconds() + 60.0
            || state.checkout.as_deref() != Some(checkout(&self.context)?.as_str())
            || state.adapter.as_deref() != Some(adapter().as_str())
            || state.scope.as_deref() != Some(self.results.scope(REASON, "agent")?.as_str())
        {
            return Err(error("scope-changed"));
        }
        Ok(())
    }
    pub fn save(&self, input: NativeSave, tasks: &mut impl TaskResolver) -> Result<Value> {
        let brain = self.results.brain()?;
        let admission = brain.lock();
        let result = match admission {
            Ok(_guard) => self.save_inner(&input, tasks),
            Err(failure)
                if !input.spooled
                    && failure.os_code.is_some_and(|code| {
                        [libc::EROFS, libc::EACCES, libc::EPERM].contains(&code)
                    }) =>
            {
                self.spool(&input)
            }
            Err(failure) => Err(failure),
        };
        let host = self
            .read_state(&input.key, None)
            .ok()
            .flatten()
            .map(|state| state.host);
        match result {
            Ok(value) if value.get("state").is_none() => Ok(value),
            other => self.observed("save", host, other),
        }
    }
    fn save_inner(&self, input: &NativeSave, tasks: &mut impl TaskResolver) -> Result<Value> {
        if !self.enabled() {
            return Err(error("governed-session-required"));
        }
        if !result_store::hex(&input.token, 32) {
            return Err(error("invalid-token"));
        }
        let brain = self.results.brain()?;
        let _guard = brain.lock()?;
        let mut slot = input.key.clone();
        let mut state = self.read_state(&input.key, None)?;
        if state
            .as_ref()
            .is_none_or(|state| state.token != input.token)
        {
            let aside = Self::aside(&input.key);
            if let Some(other) = self.read_state(&input.key, Some(&aside)).ok().flatten()
                && other.token == input.token
            {
                state = Some(other);
                slot = aside;
            }
        }
        let mut state = state.ok_or_else(|| error("request-unavailable"))?;
        if state.token != input.token
            || state.owner != self.context.actor
            || state.project != self.project()
            || !matches!(
                state.phase.as_str(),
                "requested" | "saved" | "held" | "failed"
            )
        {
            return Err(error("request-unavailable"));
        }
        let begun = state.digest.is_some();
        if state.phase != "requested" && !begun {
            return Err(error("request-unavailable"));
        }
        if begun && (state.task_uuid.is_none() || state.revision.is_none()) {
            return Err(error("invalid-state"));
        }
        self.check_scope(&state)?;
        // For the journal: a repeat of a finished save, or a begun save
        // finished after the recovery Stop marked it failed; one the agent's
        // sandbox spooled, or one that landed after a prompt moved its
        // request aside.
        let repeat = match state.phase.as_str() {
            "saved" | "held" => Some("repeat"),
            "failed" => Some("late"),
            _ => None,
        };
        let place = if input.spooled {
            Some("spooled")
        } else if slot != input.key {
            Some("in-flight")
        } else {
            None
        };
        if input.skip {
            if begun {
                return Err(error("save-already-started"));
            }
            state.phase = "omitted".into();
            self.write_state(&slot, &state)?;
            let omitted = object!({"state":"omitted","attestation":"agent"});
            return Ok(match place {
                Some(code) => noted(omitted, "omitted", code),
                None => omitted,
            });
        }
        let normalized = self.results.normalize(&input.data, !begun)?;
        if state.task_uuid.is_none()
            && normalized.progress.is_none()
            && normalized.next_steps.as_ref().is_none_or(Vec::is_empty)
            && normalized.learnings.is_empty()
        {
            return Err(error("nothing-to-record"));
        }
        let fresh = (0.0..600.0).contains(&(seconds() - state.updated));
        // Recorded at a task another session created or moved since the
        // request.
        let mut rebased = false;
        if state.task_uuid.is_none() {
            let task = match self.task(&state.task_id, tasks) {
                Ok(task) => {
                    if !fresh {
                        return Err(error("task-changed"));
                    }
                    rebased = true;
                    task
                }
                Err(error) if error.kind == ErrorKind::NotFound => {
                    tasks.provision(&state.task_id)?
                }
                Err(error) => return Err(error),
            };
            state.task_uuid = task["id"].as_str().map(String::from);
            state.revision = task["revision"].as_u64();
            self.write_state(&slot, &state)?;
        } else if !begun {
            let task = self.task(&state.task_id, tasks)?;
            if task["id"].as_str() != state.task_uuid.as_deref() {
                return Err(error("task-changed"));
            }
            if task["revision"].as_u64() != state.revision && fresh {
                state.revision = task["revision"].as_u64();
                rebased = true;
            }
        }
        let task = state
            .task_uuid
            .as_deref()
            .ok_or_else(|| error("invalid-state"))?;
        let digest = results::request_digest(task, &normalized)?;
        if state.digest.as_ref().is_some_and(|prior| prior != &digest) {
            return Err(error("content-changed"));
        }
        state.digest = Some(digest);
        self.write_state(&slot, &state)?;
        let result = self.results.record(RecordResult {
            task_id: task.into(),
            result_id: format!("native-{}", state.token),
            revision: Revision::Exact(state.revision.ok_or_else(|| error("invalid-state"))?),
            data: input.data.clone(),
            reason: REASON.into(),
            attestation: "agent".into(),
            prepared: state.scope.clone(),
            replay_only: begun,
        })?;
        let records = result["records"].as_array().cloned().unwrap_or_default();
        state.phase = if records
            .iter()
            .any(|row| matches!(row["state"].as_str(), Some("differs" | "archived-differs")))
        {
            "held"
        } else {
            "saved"
        }
        .into();
        self.write_state(&slot, &state)?;
        let submitted = normalized
            .learnings
            .iter()
            .map(|learning| results::learning_id(task, learning))
            .collect::<Result<BTreeSet<_>>>()?
            .len();
        let code = repeat.or(place).or(rebased.then_some("rebased"));
        self.journal(Some(state.host),"save",&state.phase,code,&object!({"submitted":submitted,"learnings":records.iter().filter(|row|matches!(row["state"].as_str(),Some("created"|"completed"|"updated"))).count(),"existing":records.iter().filter(|row|matches!(row["state"].as_str(),Some("existing"|"archived"))).count(),"held":records.iter().filter(|row|matches!(row["state"].as_str(),Some("differs"|"archived-differs"|"missing"))).count(),"promoted":result["promotion"]["promoted"].as_array().map(Vec::len).unwrap_or(0),"capture_decision":normalized.capture_decision}));
        Ok(result)
    }
    fn spool(&self, input: &NativeSave) -> Result<Value> {
        let mut state = self.read_state(&input.key, None)?;
        if state
            .as_ref()
            .is_none_or(|state| state.token != input.token)
        {
            state = self
                .read_state(&input.key, Some(&Self::aside(&input.key)))
                .ok()
                .flatten();
        }
        let state = state
            .filter(|state| state.token == input.token && state.phase == "requested")
            .ok_or_else(|| error("request-unavailable"))?;
        self.check_scope(&state)?;
        if !input.skip {
            self.results.normalize(&input.data, true)?;
        }
        let record = object!({"profile":"native-result-spool-v1","token":input.token,"skip":input.skip,"data":if input.skip{Value::Null}else{input.data.clone()},"created":seconds()});
        for directory in spool_directories() {
            if fs::ensure_directory(&directory, true).is_err() || !owned(&directory) {
                continue;
            }
            if fs::atomic_json(&directory.join(format!("{}.json", input.key)), &record).is_ok() {
                return Ok(
                    object!({"state":"spooled","attestation":"agent","detail":"This sandbox cannot write the project; the save is made when this turn ends."}),
                );
            }
        }
        Err(error("project-read-only"))
    }
    pub fn drain(
        &self,
        input: NativeDrain,
        tasks: &mut impl TaskResolver,
    ) -> Result<Option<Value>> {
        let task = match results::task_id(&input.task_id) {
            Ok(task) => task,
            Err(_) => return Ok(None),
        };
        let Some(key) = self.key(&task, input.host, &input.payload)? else {
            return Ok(None);
        };
        let mut result = None;
        for directory in spool_directories() {
            if !directory.is_dir() || !owned(&directory) {
                continue;
            }
            // Avoid changing an unrelated fallback spool's shared lock merely
            // because it exists. Only this session's pending records need custody.
            // A record arriving after this bounded scan waits for the next hook.
            let pending = (0..=3).any(|attempt| {
                let name = if attempt == 0 {
                    format!("{key}.json")
                } else {
                    format!("{key}.attempt{attempt}.json")
                };
                let path = directory.join(name);
                owned(&path) && std::fs::symlink_metadata(&path).is_ok_and(|image| image.is_file())
            });
            if !pending {
                continue;
            }
            let Some(_guard) = DrainGuard::acquire(&directory, "spool")? else {
                continue;
            };
            let names = (1..=3)
                .rev()
                .map(|attempt| {
                    (
                        directory.join(format!("{key}.attempt{attempt}.json")),
                        attempt,
                    )
                })
                .chain(std::iter::once((directory.join(format!("{key}.json")), 0)));
            for (path, begun) in names {
                if !owned(&path) {
                    continue;
                }
                let Some(record) = result_store::read_strict_json(&path, MAX_INPUT + 4096)
                    .ok()
                    .flatten()
                else {
                    continue;
                };
                if record["profile"] != "native-result-spool-v1"
                    || !record["skip"].is_boolean()
                    || !result_store::hex(record["token"].as_str().unwrap_or(""), 32)
                {
                    continue;
                }
                if begun >= 3 {
                    if std::fs::remove_file(&path).is_ok() {
                        self.journal(
                            Some(input.host),
                            "save",
                            "refused",
                            Some("interrupted"),
                            &Value::Null,
                        );
                    }
                    continue;
                }
                let attempt = directory.join(format!("{key}.attempt{}.json", begun + 1));
                if std::fs::rename(&path, &attempt).is_err() {
                    continue;
                }
                match self.save(
                    NativeSave {
                        key: key.clone(),
                        token: record["token"].as_str().unwrap_or("").into(),
                        data: record["data"].clone(),
                        skip: record["skip"] == true,
                        spooled: true,
                    },
                    tasks,
                ) {
                    Ok(value) => result = Some(value),
                    Err(error) if error.kind == ErrorKind::Io => return Err(error),
                    Err(_) => {}
                }
                let _ = std::fs::remove_file(attempt);
            }
        }
        Ok(result)
    }
    fn prune_states(&self, key: &str) -> Result<()> {
        let path = self.state_path(key)?;
        let Some(parent) = path.parent() else {
            return Ok(());
        };
        let aside_path = self.state_path(&Self::aside(key))?;
        let mut files = std::fs::read_dir(parent)?
            .filter_map(std::result::Result::ok)
            .filter(|entry| {
                entry.path() != path
                    && entry.path() != aside_path
                    && entry
                        .path()
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .is_some_and(|value| result_store::hex(value, 64))
            })
            .collect::<Vec<_>>();
        files.sort_by_key(|entry| {
            entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
        });
        let remove = files.len().saturating_sub(MAX_STATES - 1);
        for entry in files.into_iter().take(remove) {
            if !fs::linked_at(&entry.path(), &std::fs::symlink_metadata(entry.path())?)? {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        Ok(())
    }
    fn instruction(
        &self,
        key: &str,
        state: &NativeState,
        payload: &Value,
        entered: bool,
    ) -> Result<(String, bool)> {
        let executable = std::env::current_exe()?;
        let roots = &self.context.roots;
        let named = roots.shared || entered;
        let mut arguments = format!(" memory --root {}", quote(&roots.state.to_string_lossy()));
        if named {
            arguments.push_str(&format!(
                " --project {}",
                quote(&roots.project.to_string_lossy())
            ));
        }
        arguments.push_str(&format!(
            " native-result --request {} --token {}",
            key, state.token
        ));
        let (ask, skip) = if state.host == Host::Claude {
            // Claude Code asks before every command its allow rules do not
            // cover, and before any subshell, { } group, heredoc with braces
            // and quotes, or printf operand holding [ with $ or a backtick,
            // whatever the rules say. A cd into the installation, echo -E and
            // the installed `memory-bank/bin/infra-engine memory` spelling
            // pass; -E keeps the JSON's backslashes in zsh as in bash. An
            // absolute engine path matches no installed rule: the save then
            // asked after every answer and was refused under `claude -p`.
            let program = claude_program(&roots.tooling, &executable);
            let enter = format!("cd {} && ", quote(&roots.tooling.to_string_lossy()));
            // Once a session has entered a worktree, Claude Code refuses a
            // program fed a descriptor it names ("-"): there the engine reads
            // the piped result without --input -.
            let piped = if named { "" } else { " --input -" };
            // Attached, the cd is into the clone the launch adds as a working
            // directory; a save that went through returns to where the
            // session was, a refused one keeps its exit status.
            let back = payload["cwd"]
                .as_str()
                .filter(|cwd| {
                    roots.attached()
                        && !roots.shared
                        && Path::new(cwd).is_absolute()
                        && !cwd.contains(['\n', '\r', '\0'])
                })
                .map(|cwd| format!(" && cd {}", quote(cwd)))
                .unwrap_or_default();
            (
                format!(
                    "Use your shell tool to run this exact command: {enter}echo -E '<JSON>' | {program}{arguments}{piped} --json{back}\nPut one JSON object in place of <JSON>, writing each ' inside it as '\\''. "
                ),
                format!(
                    "If no safe result exists, run this command instead: {enter}{program}{arguments} --skip --json{back}\n"
                ),
            )
        } else {
            (
                format!(
                    "Use your shell tool to pipe one JSON object into this exact command: {}{arguments} --input - --json\n",
                    quote(&executable.to_string_lossy())
                ),
                "If no safe result exists, replace --input - with --skip. ".into(),
            )
        };
        let answer = self
            .setting(
                "CONTEXT_NATIVE_RESULT_ANSWER",
                "native_result_answer",
                "auto",
                &["auto", "repeat", "brief"],
            )
            .unwrap_or_else(|_| "repeat".into());
        let brief = answer == "brief" || answer == "auto" && attended(state.host, payload);
        let review = crate::review::ReviewService::new(self.context.clone());
        let proposal = review.maintenance_mode()? == "review-verified";
        let config = RuntimeConfig::load(&self.context.roots)?;
        let configured = std::env::var("CONTEXT_NATIVE_RESULT_INSTRUCTION")
            .ok()
            .or_else(|| {
                config.value["native_result_instruction"]
                    .as_str()
                    .map(String::from)
            })
            .unwrap_or_else(|| "auto".into());
        let marker = format!(
            "Native result reference schema v1. Capture policy {}.",
            if proposal {
                "proposal-only"
            } else {
                "ordinary"
            }
        );
        let full = configured != "auto"
            || state.host == Host::Cursor
            || !rules_in_context(payload, &marker);
        let mut body = format!(
            // The full step keeps the wording measured with live models before
            // the native runtime: the schema example, when verified=true is
            // honest (it promotes to durable memory), no duplicate findings,
            // and where the checked fingerprints come from.
            "This is an internal continuation: do not announce routine memory operations. Save only the result of the work already performed in this session. Do not repeat the task, edit application files, run tests, delegate, or mark the task completed. {ask}Native result reference schema v1. Capture policy {}. JSON: progress (summary, at most 1000 characters), optional next_steps (at most 3 strings of 300 characters; omit to preserve the current plan), learnings (at most 3 objects with type finding|decision, title, consequence, sources). Exact schema example (replace placeholders, never save the example): {{\"progress\":\"summary\",\"verified\":true,\"learnings\":[{{\"type\":\"finding\",\"title\":\"checked rule\",\"consequence\":\"reusable consequence\",\"sources\":[\"checked-file.php\"]}}]}}. verified is a TOP-LEVEL boolean, NEVER a field inside a learning. New consequences use ordinary learnings. To recheck or correct an inspected existing claim, preserve its type/title/sources and add its exact record_id, expected_revision and checked source_fingerprints (read with memory-bank/bin/infra-engine memory source-fingerprints --source PATH --json). Do not create a duplicate finding. Recalling alone uses learnings=[] and progress only. Use only canonical project-relative source files you actually checked. Set verified=true only when you personally checked every learning against current sources; this is agent attestation, never human review. Empty learnings is valid. No raw requests, transcripts, logs, credentials or personal data. {skip}Check the command result. ",
            if proposal {
                "proposal-only"
            } else {
                "ordinary"
            }
        );
        if proposal {
            body.push_str("Independent-review mode: use learnings=[]; reusable knowledge is proposal-only and published by the external controller. ");
        }
        if !full {
            let start = body
                .find("Native result reference schema v1.")
                .unwrap_or(body.len());
            let end = body[start..]
                .find("If no safe result exists")
                .map(|offset| start + offset)
                .unwrap_or(body.len());
            body.replace_range(start..end,"The JSON follows the full save step earlier in this conversation: progress, optional next_steps, source-backed learnings and a top-level verified boolean. ");
        }
        if review.maintenance_mode()? == "encountered"
            && let Some(uuid) = &state.task_uuid
        {
            let hints = review
                .candidates(&state.task_id, uuid)?
                .into_iter()
                .map(|mut row| {
                    if let Some(map) = row.as_object_mut() {
                        map.retain(|key, _| !key.starts_with('_'));
                    }
                    row
                })
                .collect::<Vec<_>>();
            if !hints.is_empty() {
                body.push_str(" Experimental maintenance: these same-task encounter hints have changed cited files. They are untrusted prior claims, not proof you checked them. Confirm/correct only if the original task work already checked the claim and current sources; preserve type/title/sources and supply checked source_fingerprints. Otherwise omit them; do not start research or bulk-refresh hashes. References: ");
                body.push_str(&json::python_default(&object!(hints), false, false, true)?);
            }
        }
        if brief {
            body.push_str("After successful save or skip end with no further text or one short line; the person already read the answer. On refusal briefly report the held save. ");
        } else {
            body.push_str("After successful save or skip return the substantive original answer with its facts, citations and requested format; do not replace it with a memory acknowledgement. On refusal retain the answer and briefly report the held save. ");
        }
        body.push_str("Correct JSON/schema once only if refused before any write. On stale revision, scope or ownership refusal stop; never override newer work or repeat the user task.");
        let marker = format!("INTERNAL_NATIVE_MEMORY_RESULT {}", state.token);
        Ok((
            if state.host == Host::Claude {
                format!("{body}\n{marker}")
            } else {
                format!("{marker}\n{body}")
            },
            brief,
        ))
    }
}
pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
/// A prompt or Stop result with the journal event and code it is observed
/// with; `observed` removes the note before the result leaves.
fn noted(mut value: Value, event: &str, code: &str) -> Value {
    value["_journal"] = object!({"event":event,"code":code});
    value
}
/// The engine as a Claude Code save names it: the spelling the installed
/// `Bash(memory-bank/bin/infra-engine memory:*)` rule allows after a cd into
/// the installation, when the engine serving this request is the one installed
/// there. Another engine - an attached session's, say - keeps its absolute
/// path: the installed one could be a different build.
pub fn claude_program(tooling: &Path, executable: &Path) -> String {
    let name = if cfg!(windows) {
        "memory-bank/bin/infra-engine.exe"
    } else {
        "memory-bank/bin/infra-engine"
    };
    let installed = fs::canonicalize(tooling.join(name)).ok();
    if installed.is_some() && installed == fs::canonicalize(executable).ok() {
        name.into()
    } else {
        quote(&executable.to_string_lossy())
    }
}
pub fn recovery(host: Host, payload: &Value) -> bool {
    if host == Host::Cursor {
        payload["loop_count"]
            .as_u64()
            .is_some_and(|count| count > 0)
    } else {
        payload["stop_hook_active"] == true
    }
}
pub fn tail(payload: &Value, limit: usize) -> Option<Vec<u8>> {
    let path = payload["transcript_path"].as_str()?;
    let mut file = std::fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let size = file.seek(SeekFrom::End(0)).ok()?;
    file.seek(SeekFrom::Start(size.saturating_sub(limit as u64)))
        .ok()?;
    let mut raw = vec![];
    file.take(limit as u64).read_to_end(&mut raw).ok()?;
    Some(raw)
}
fn rules_in_context(payload: &Value, marker: &str) -> bool {
    let Some(raw) = tail(payload, 2 * 1024 * 1024) else {
        return false;
    };
    let mut begin = 0;
    for record in [
        b"\"subtype\":\"compact_boundary\"".as_slice(),
        b"\"type\":\"compacted\"".as_slice(),
    ] {
        if let Some(position) = raw
            .windows(record.len())
            .rposition(|window| window == record)
        {
            let next = raw[position..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|offset| position + offset + 1)
                .unwrap_or(raw.len());
            begin = begin.max(next);
        }
    }
    let full = b"Native result reference schema v1.";
    raw[begin..]
        .windows(full.len())
        .rposition(|window| window == full)
        .is_some_and(|offset| raw[begin + offset..].starts_with(marker.as_bytes()))
}
fn rollout_turn(payload: &Value) -> Value {
    let Some(turn) = payload["turn_id"].as_str() else {
        return Value::Null;
    };
    let Some(raw) = tail(payload, 512 * 1024) else {
        return Value::Null;
    };
    for line in raw.split(|byte| *byte == b'\n').rev() {
        if let Ok(value) = serde_json::from_slice::<Value>(line)
            && value["type"] == "turn_context"
        {
            return if value["payload"]["turn_id"] == turn {
                value["payload"].clone()
            } else {
                Value::Null
            };
        }
    }
    Value::Null
}
fn claude_refusal(payload: &Value, token: &str) -> bool {
    let Some(raw) = tail(payload, 512 * 1024) else {
        return false;
    };
    let mut calls = BTreeSet::new();
    let mut refused = false;
    for line in raw.split(|byte| *byte == b'\n') {
        let Ok(record) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        for block in record["message"]["content"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if block["type"] == "tool_use" && block["input"].to_string().contains(token) {
                if let Some(id) = block["id"].as_str() {
                    calls.insert(id.to_string());
                }
            } else if block["type"] == "tool_result"
                && block["tool_use_id"]
                    .as_str()
                    .is_some_and(|id| calls.contains(id))
            {
                refused = matches!(
                    record["toolDenialKind"].as_str(),
                    Some("user-rejected" | "permission-rule" | "automode-blocked")
                );
            }
        }
    }
    refused
}
fn attended(host: Host, payload: &Value) -> bool {
    match host {
        Host::Cursor => false,
        Host::Claude => {
            std::env::var("CLAUDE_CODE_SESSION_ATTENDED").as_deref() == Ok("1")
                && matches!(
                    std::env::var("CLAUDE_CODE_ENTRYPOINT").as_deref(),
                    Ok("cli" | "claude-vscode" | "claude-desktop")
                )
        }
        Host::Codex => {
            let Some(path) = payload["transcript_path"].as_str() else {
                return false;
            };
            let Ok(file) = std::fs::File::open(path) else {
                return false;
            };
            let mut raw = Vec::new();
            if file.take(256 * 1024 + 1).read_to_end(&mut raw).is_err() {
                return false;
            }
            let first = raw.split(|byte| *byte == b'\n').next().unwrap_or(&[]);
            serde_json::from_slice::<Value>(first).is_ok_and(|value| {
                value["type"] == "session_meta"
                    && matches!(value["payload"]["source"].as_str(), Some("cli" | "vscode"))
            })
        }
    }
}
pub fn checkout(context: &RuntimeContext) -> Result<String> {
    let Some(frame) = crate::git::GitFrame::discover(&context.roots.project)? else {
        return Ok(result_store::sha(b"no-git-head"));
    };
    let args = crate::git::strings(&[
        "rev-parse",
        "--absolute-git-dir",
        "HEAD",
        "--symbolic-full-name",
        "HEAD",
    ]);
    let output = crate::git::run(
        &context.roots.project,
        &args,
        None,
        std::time::Duration::from_secs(2),
        64 * 1024,
    )?;
    if output.code != 0 {
        return Ok(result_store::sha(b"no-git-head"));
    }
    let lines = output
        .stdout
        .split(|byte| *byte == b'\n')
        .collect::<Vec<_>>();
    let name = lines.get(2).copied().unwrap_or(&[]);
    if name.starts_with(b"refs/heads/") {
        return Ok(result_store::sha(name));
    }
    let owner = result_store::read_json(&frame.admin.join("codex-thread.json"), 4096)
        .ok()
        .flatten();
    if let Some(owner) = owner
        && owner["version"] == 1
        && let Some(id) = owner["ownerThreadId"].as_str()
        && uuid::Uuid::parse_str(id).is_ok()
    {
        return Ok(result_store::sha(format!("codex-thread/{id}").as_bytes()));
    }
    Ok(result_store::sha(lines.get(1).copied().unwrap_or(&[])))
}
fn sandbox_writes(policy: &Value, target: &Path) -> bool {
    let target = fs::canonical_or_missing(target).unwrap_or_else(|_| target.to_path_buf());
    let mut best: Option<(usize, bool)> = None;
    for entry in policy["entries"].as_array().into_iter().flatten() {
        let source = if entry["path"]["type"] == "path" {
            entry["path"]["path"].as_str().map(PathBuf::from)
        } else {
            match entry["path"]["value"]["kind"].as_str() {
                Some("root") => Some(PathBuf::from(std::path::MAIN_SEPARATOR.to_string())),
                Some("slash_tmp") => Some(PathBuf::from("/tmp")),
                Some("tmpdir") => Some(std::env::temp_dir()),
                _ => None,
            }
        };
        let Some(source) = source else {
            continue;
        };
        let source = fs::canonical_or_missing(&source).unwrap_or(source);
        if target.starts_with(&source) {
            let specificity = source.as_os_str().len();
            if best.is_none_or(|(previous, _)| specificity > previous) {
                best = Some((specificity, entry["access"] == "write"));
            }
        }
    }
    best.is_some_and(|(_, write)| write)
}
pub fn spool_directories() -> Vec<PathBuf> {
    #[cfg(unix)]
    let uid = unsafe { libc::getuid() }.to_string();
    #[cfg(not(unix))]
    let uid = "user".to_string();
    let mut bases = vec![];
    if let Some(base) = std::env::var_os("TMPDIR") {
        let base = PathBuf::from(base);
        if base.is_absolute() {
            bases.push(base);
        }
    }
    bases.push(std::env::temp_dir());
    #[cfg(unix)]
    bases.push(PathBuf::from("/tmp"));
    bases.dedup();
    let claude = std::env::var_os("CLAUDE_CODE_TMPDIR").map(PathBuf::from);
    let mut children = claude
        .into_iter()
        .chain(bases.clone())
        .map(|base| base.join(format!("claude-{}", if uid == "user" { "0" } else { &uid })))
        .collect::<Vec<_>>();
    bases.append(&mut children);
    let mut unique = BTreeSet::new();
    bases
        .into_iter()
        .filter(|base| base.is_absolute() && unique.insert(base.clone()))
        .map(|base| base.join(format!("ai-accelerator-native-spool-{uid}")))
        .collect()
}
pub fn owned(path: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if fs::linked_at(path, &metadata).unwrap_or(true) {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.uid() == unsafe { libc::getuid() }
    }
    #[cfg(not(unix))]
    {
        true
    }
}
pub struct DrainGuard(std::fs::File);
impl DrainGuard {
    pub fn acquire(folder: &Path, name: &str) -> Result<Option<Self>> {
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
        }
        let file = match options.open(folder.join(format!(".{name}.drain-lock"))) {
            Ok(file) => file,
            Err(_) => return Ok(None),
        };
        if !file.metadata()?.is_file() {
            return Ok(None);
        }
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Ok(None);
            }
        }
        #[cfg(windows)]
        {
            if !windows_lock::lock(&file, false) {
                return Ok(None);
            }
        }
        Ok(Some(Self(file)))
    }
}
impl Drop for DrainGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
        }
        #[cfg(windows)]
        {
            windows_lock::lock(&self.0, true);
        }
    }
}
#[cfg(windows)]
mod windows_lock {
    use std::os::windows::io::AsRawHandle;
    #[repr(C)]
    struct Overlapped {
        internal: usize,
        high: usize,
        offset: u32,
        offset_high: u32,
        event: *mut std::ffi::c_void,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LockFileEx(
            handle: *mut std::ffi::c_void,
            flags: u32,
            reserved: u32,
            low: u32,
            high: u32,
            overlap: *mut Overlapped,
        ) -> i32;
        fn UnlockFileEx(
            handle: *mut std::ffi::c_void,
            reserved: u32,
            low: u32,
            high: u32,
            overlap: *mut Overlapped,
        ) -> i32;
    }
    pub fn lock(file: &std::fs::File, unlock: bool) -> bool {
        let mut overlap = Overlapped {
            internal: 0,
            high: 0,
            offset: 0,
            offset_high: 0,
            event: std::ptr::null_mut(),
        };
        unsafe {
            if unlock {
                UnlockFileEx(file.as_raw_handle(), 0, 1, 0, &mut overlap) != 0
            } else {
                LockFileEx(file.as_raw_handle(), 3, 0, 1, 0, &mut overlap) != 0
            }
        }
    }
}
