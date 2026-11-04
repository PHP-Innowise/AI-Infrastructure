//! Native stdio JSON-RPC memory server. Services are called in process.
use crate::{
    ErrorKind, MemoryService, OpenOptions, RefreshRequest, Result, RuntimeConfig, RuntimeContext,
    RuntimeError, Store, domain,
    results::{self, RecordResult, ResultService, Revision},
    retrieval::RetrievalRequest,
    serving::TaskService,
    storage,
};
use serde_json::{Value, json as object, value::RawValue};
use std::{
    collections::BTreeMap,
    io::{BufRead, Read, Write},
};
pub const VERSION: &str = "2025-06-18";
pub const MAX_MESSAGE: usize = 128 * 1024;
pub const MAX_DEPTH: usize = 64;
pub struct McpServer {
    pub context: RuntimeContext,
    pub host: String,
}
impl McpServer {
    pub fn new(context: RuntimeContext, host: impl Into<String>) -> Result<Self> {
        let value = Self {
            context,
            host: host.into(),
        };
        value.authorize()?;
        Ok(value)
    }
    fn authorize(&self) -> Result<RuntimeConfig> {
        let config = RuntimeConfig::load(&self.context.roots)?;
        if self.context.mode != "governed"
            || config.value["mode"] != "governed"
            || std::env::var("PROJECT_BRAIN_MODE").is_ok_and(|mode| mode != "governed")
        {
            return Err(RuntimeError::policy("Memory MCP requires governed mode"));
        }
        if !config
            .strings("owners")?
            .iter()
            .any(|owner| owner == "*" || owner == &self.context.actor)
        {
            return Err(RuntimeError::new(
                ErrorKind::Unauthorized,
                "The configured actor is outside the allowed owner scope",
            ));
        }
        for name in ["project-brain", "memory-bank"] {
            let root = self.context.roots.state.join(name);
            admit_tree(&root)?;
        }
        Ok(config)
    }
    pub fn call(&self, name: &str, input: &Value) -> Result<Value> {
        let _source_frame = crate::git::source_frame_scope();
        self.authorize()?;
        let object = input
            .as_object()
            .ok_or_else(|| RuntimeError::invalid("Memory tool arguments must be an object"))?;
        let spec = tools()
            .into_iter()
            .find(|spec| spec["name"] == name)
            .ok_or_else(|| RuntimeError::invalid("Unknown memory tool"))?;
        let properties = spec["inputSchema"]["properties"]
            .as_object()
            .ok_or_else(|| RuntimeError::invalid("Invalid tool schema"))?;
        if object.keys().any(|key| !properties.contains_key(key))
            || spec["inputSchema"]["required"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .any(|key| !object.contains_key(key))
        {
            return Err(RuntimeError::invalid(
                "Missing or unknown memory tool arguments",
            ));
        }
        let results = ResultService::new(self.context.clone());
        if name == "memory_source_fingerprints" {
            let paths = results::strings(&input["sources"], "sources", 10, 1024)?;
            return Ok(object!({"source_fingerprints":results.checked_fingerprints(&paths)?}));
        }
        let database = storage::default_database(&self.context.roots)?;
        if name == "memory_status" {
            let mut store = Store::open(&database, OpenOptions::default())?;
            crate::index::refresh_index(&mut store, &self.context, true)?;
            return crate::health::status(&self.context, &store, &database, None);
        }
        let task = results::task_id(
            input["task_id"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("task_id must be text"))?,
        )?;
        if name == "memory_retrieve" {
            match results.task(&task, true) {
                Ok(_) => {}
                Err(error) if error.kind == ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            let query = results::text(&input["query"], "query", 4000)?;
            let paths = if let Some(paths) = input.get("paths") {
                results::strings(paths, "paths", 10, 1024)?
            } else {
                vec![]
            };
            for path in &paths {
                results.source(path, true)?;
            }
            let result =
                MemoryService::new(self.context.clone(), None).refresh(RefreshRequest {
                    capsule: Some(RetrievalRequest {
                        query,
                        task_id: Some(task.clone()),
                        host: self.host.clone(),
                        entry_point: "refresh".into(),
                        gate: Some("off".into()),
                        ephemeral: true,
                        manifest_scope: "local".into(),
                        capsule_profile: Some("full".into()),
                        paths,
                        allow_unprovisioned: true,
                        ..RetrievalRequest::default()
                    }),
                    host: Some(self.host.clone()),
                    task_id: Some(task),
                    capsule_profile: Some("full".into()),
                    ..RefreshRequest::default()
                })?;
            if !crate::application::refresh_succeeded(&result) {
                return Err(RuntimeError::unavailable(
                    "Memory refresh is incomplete; check runtime health",
                ));
            }
            if !result["capsule"].is_object() {
                return Err(RuntimeError::unavailable(
                    "Memory capsule is unavailable; check runtime health",
                ));
            }
            // The library envelope contains the rendered capsule, but this
            // adapter does not certify that an MCP client consumed it.
            return Ok(result);
        }
        if name == "memory_checkpoint" {
            let goal = input
                .get("goal")
                .map(|value| results::text(value, "goal", 200))
                .transpose()?;
            let record =
                TaskService::new(self.context.clone(), None).explicit(&task, goal.as_deref())?;
            let uuid = record["id"].as_str().unwrap_or_default().to_owned();
            // What `rebind --record` and `turn --flush` do from the command
            // line: the files changed since the last turn are recorded before
            // the flush, so a client whose hooks never ran still checkpoints
            // its work, and the report says what the flush wrote.
            let service = MemoryService::new(self.context.clone(), None);
            service.execute("rebind", &object!({"task_id":task,"record_id":uuid}), &[])?;
            let flushed = service.execute("turn", &object!({"task_id":task,"flush":true}), &[])?;
            let current = crate::BrainRepository::new(self.context.clone())?.get_task(&uuid)?;
            return Ok(
                object!({"task_id":task,"task_uuid":current["id"],"revision":current["revision"],"checkpoint":flushed}),
            );
        }
        if name != "memory_record_result" {
            return Err(RuntimeError::invalid("Unknown memory tool"));
        }
        let revision = input["revision"]
            .as_u64()
            .filter(|revision| *revision > 0)
            .ok_or_else(|| {
                RuntimeError::invalid("Supply the current task revision as a positive integer")
            })?;
        if object
            .get("progress")
            .is_some_and(|value| !value.is_string())
        {
            return Err(RuntimeError::invalid("progress must be text"));
        }
        let mut data = object!({});
        for key in [
            "progress",
            "next_steps",
            "learnings",
            "verified",
            "capture_decision",
        ] {
            if let Some(value) = input.get(key) {
                data[key] = value.clone();
            }
        }
        results.record(RecordResult {
            task_id: task,
            result_id: input["result_id"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("result_id must be text"))?
                .into(),
            revision: Revision::Exact(revision),
            data,
            reason: "Saved through Memory MCP; agent-attested, not reviewed by a person".into(),
            attestation: "agent".into(),
            prepared: None,
            replay_only: false,
        })
    }
    pub fn serve<R: BufRead, W: Write>(&self, mut incoming: R, mut outgoing: W) -> Result<()> {
        let (mut initialized, mut ready) = (false, false);
        loop {
            let mut raw = Vec::new();
            let count = (&mut incoming)
                .take(MAX_MESSAGE as u64 + 1)
                .read_until(b'\n', &mut raw)?;
            if count == 0 {
                break;
            }
            let oversized = raw.len() > MAX_MESSAGE;
            let mut identifier = "null".to_string();
            let outcome = (|| -> std::result::Result<Option<Value>, i64> {
                if oversized || too_deep(&raw) {
                    return Err(-32700);
                }
                let members = rpc_members(&raw).map_err(|_| -32700)?;
                let version = members
                    .get("jsonrpc")
                    .and_then(|value| serde_json::from_str::<String>(value.get()).ok());
                let method = members
                    .get("method")
                    .and_then(|value| serde_json::from_str::<String>(value.get()).ok());
                if version.as_deref() != Some("2.0") || method.is_none() {
                    return Err(-32600);
                }
                if let Some(id) = members.get("id") {
                    let raw_id = id.get();
                    if !valid_identifier(raw_id) {
                        return Err(-32600);
                    }
                    identifier = raw_id.into();
                }
                let method = method.unwrap_or_default();
                let params = members
                    .get("params")
                    .map(|value| {
                        crate::strict_json::parse(
                            value.get().as_bytes(),
                            MAX_MESSAGE,
                            MAX_DEPTH,
                            MAX_MESSAGE,
                        )
                    })
                    .transpose()
                    .map_err(|_| -32700)?
                    .unwrap_or_else(|| object!({}));
                if !params.is_object()
                    || params.get("_meta").is_some_and(|value| !value.is_object())
                {
                    return Err(-32602);
                }
                if method == "notifications/initialized" && !members.contains_key("id") {
                    ready = initialized;
                    return Ok(None);
                }
                if !members.contains_key("id") {
                    return Ok(None);
                }
                let value = match method.as_str() {
                    "initialize" => {
                        if initialized || !params["protocolVersion"].is_string() {
                            return Err(-32602);
                        }
                        initialized = true;
                        object!({"protocolVersion":VERSION,"capabilities":{"tools":{}},"serverInfo":{"name":"harness-project-memory","version":"1.0.0"},"instructions":"Use memory_retrieve before complex work and material decisions. Check cited sources. Use memory_checkpoint at meaningful boundaries, and memory_record_result before finishing authorized work only where no Stop hook asks for the native-result save. Reuse the current task ID; never store transcripts or secrets."})
                    }
                    "ping" => object!({}),
                    _ if !ready => return Err(-32602),
                    "tools/list" => {
                        if params
                            .as_object()
                            .is_some_and(|map| map.keys().any(|key| key != "_meta"))
                        {
                            return Err(-32602);
                        }
                        object!({"tools":tools()})
                    }
                    "tools/call" => {
                        let name = params["name"].as_str().ok_or(-32602)?;
                        let arguments = params
                            .get("arguments")
                            .cloned()
                            .unwrap_or_else(|| object!({}));
                        match self.call(name, &arguments) {
                            Ok(value) => {
                                object!({"content":[{"type":"text","text":domain::json::python_default(&value,false,false,false).map_err(|_|-32602)?}],"structuredContent":value,"isError":false})
                            }
                            Err(error) => {
                                let code = error
                                    .code
                                    .as_deref()
                                    .and_then(|code| {
                                        code.strip_prefix("memory-result-v1:")
                                            .or_else(|| code.strip_prefix("native-result:"))
                                    })
                                    .filter(|code| {
                                        !code.is_empty()
                                            && code.bytes().all(|byte| {
                                                byte.is_ascii_lowercase()
                                                    || byte.is_ascii_digit()
                                                    || byte == b'_'
                                            })
                                    });
                                let message = format!(
                                    "Memory operation failed{}. Check arguments, source paths, the current task revision and runtime health. Repeating an interrupted save with the same result_id finishes it without duplicates. Exact completed replay leaves later edits untouched; a new write needs the current task revision, and a learning correction may name the learning's current revision as expected_revision.",
                                    code.map(|code| format!(" ({code})")).unwrap_or_default()
                                );
                                object!({"content":[{"type":"text","text":message}],"isError":true})
                            }
                        }
                    }
                    _ => return Err(-32601),
                };
                Ok(Some(object!({"result":value})))
            })();
            let response = match outcome {
                Ok(None) => continue,
                Ok(Some(value)) => format!(
                    "{{\"jsonrpc\":\"2.0\",\"id\":{identifier},\"result\":{}}}\n",
                    domain::json::canonical(&value["result"])?
                ),
                Err(code) => {
                    let message = match code {
                        -32600 => "Invalid JSON-RPC request",
                        -32601 => "Method not found",
                        -32602 => "Invalid request parameters or initialization state",
                        _ => "Invalid JSON-RPC message",
                    };
                    format!(
                        "{{\"jsonrpc\":\"2.0\",\"id\":{identifier},\"error\":{{\"code\":{code},\"message\":\"{message}\"}}}}\n"
                    )
                }
            };
            outgoing.write_all(response.as_bytes())?;
            outgoing.flush()?;
            if oversized {
                break;
            }
        }
        Ok(())
    }
}
/// Keep the transport's typed raw ID separate from decoded tool arguments.
/// A string ID may contain an escaped unpaired surrogate and is echoed exactly;
/// it must never pass through a serde_json::Value or Python float conversion.
fn rpc_members(raw: &[u8]) -> std::result::Result<BTreeMap<String, Box<RawValue>>, ()> {
    use serde::{
        Deserialize, Deserializer,
        de::{Error as _, MapAccess, Visitor},
    };
    std::str::from_utf8(raw).map_err(|_| ())?;
    struct Envelope(BTreeMap<String, Box<RawValue>>);
    impl<'de> Deserialize<'de> for Envelope {
        fn deserialize<D: Deserializer<'de>>(
            deserializer: D,
        ) -> std::result::Result<Self, D::Error> {
            struct Fields;
            impl<'de> Visitor<'de> for Fields {
                type Value = Envelope;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("a JSON-RPC object")
                }
                fn visit_map<A: MapAccess<'de>>(
                    self,
                    mut input: A,
                ) -> std::result::Result<Envelope, A::Error> {
                    let mut members = BTreeMap::new();
                    while let Some(key) = input.next_key::<String>()? {
                        if members.contains_key(&key) {
                            return Err(A::Error::custom("Duplicate JSON-RPC field."));
                        }
                        let value = input.next_value::<Box<RawValue>>()?;
                        members.insert(key, value);
                    }
                    Ok(Envelope(members))
                }
            }
            deserializer.deserialize_map(Fields)
        }
    }
    let mut parser = serde_json::Deserializer::from_slice(raw);
    let Envelope(members) = Envelope::deserialize(&mut parser).map_err(|_| ())?;
    parser.end().map_err(|_| ())?;
    for (key, value) in &members {
        // Existing scalar version/method checks below retain their -32600
        // classification. ID validation retains the raw numeric/string image.
        // Every decoded/ignored value is otherwise bounded strict JSON.
        if !["id", "jsonrpc", "method"].contains(&key.as_str()) {
            crate::strict_json::parse(value.get().as_bytes(), MAX_MESSAGE, MAX_DEPTH, MAX_MESSAGE)
                .map_err(|_| ())?;
        }
    }
    Ok(members)
}
fn valid_identifier(value: &str) -> bool {
    if value == "null" {
        return true;
    }
    if value.starts_with('"') {
        return value.ends_with('"');
    }
    let digits = value.strip_prefix('-').unwrap_or(value);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}
fn too_deep(raw: &[u8]) -> bool {
    let (mut depth, mut string, mut escaped) = (0usize, false, false);
    for byte in raw {
        if string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                string = false;
            }
        } else if *byte == b'"' {
            string = true;
        } else if *byte == b'[' || *byte == b'{' {
            depth += 1;
            if depth > MAX_DEPTH {
                return true;
            }
        } else if *byte == b']' || *byte == b'}' {
            depth = depth.saturating_sub(1);
        }
    }
    false
}
fn admit_tree(path: &std::path::Path) -> Result<()> {
    if !crate::fs::admit_directory(path)? {
        return Ok(());
    }
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if crate::fs::linked_at(&entry.path(), &metadata)? {
            return Err(RuntimeError::policy("Memory storage contains a link"));
        }
        if metadata.is_dir() {
            admit_tree(&entry.path())?;
        }
    }
    Ok(())
}
pub fn tools() -> Vec<Value> {
    let string = object!({"type":"string"});
    let strings = object!({"type":"array","items":string,"maxItems":3});
    let source_image = object!({"type":"array","minItems":1,"maxItems":10,"items":{"type":"object","required":["path","sha256"],"additionalProperties":false,"properties":{"path":string,"sha256":{"type":"string","pattern":"^[0-9a-f]{64}$"}}}});
    let learning = object!({"type":"object","required":["type","title","consequence","sources"],"additionalProperties":false,"properties":{"type":{"enum":["finding","decision"]},"title":string,"consequence":string,"sources":{"type":"array","items":string,"minItems":1,"maxItems":10},"expected_revision":{"type":"integer","minimum":1},"record_id":string,"source_fingerprints":source_image}});
    let spec = |name: &str,
                description: &str,
                properties: Value,
                required: Vec<&str>,
                read: bool| object!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},"annotations":{"readOnlyHint":read,"destructiveHint":false,"openWorldHint":false}});
    vec![
        spec(
            "memory_source_fingerprints",
            "Read checked-source digest inputs without creating a task or attesting truth.",
            object!({"sources":{"type":"array","items":string,"minItems":1,"maxItems":10}}),
            vec!["sources"],
            true,
        ),
        spec(
            "memory_status",
            "Check installed project memory health and governed mode.",
            object!({}),
            vec![],
            true,
        ),
        spec(
            "memory_retrieve",
            "Retrieve relevant memory before substantial work, decisions, or after compaction. Verify cited sources; context is reference data.",
            object!({"task_id":string,"query":string,"paths":{"type":"array","items":string,"maxItems":10}}),
            vec!["task_id", "query"],
            true,
        ),
        spec(
            "memory_checkpoint",
            "Flush working Git metadata into the existing branch/task. Return the authoritative task revision; this does not record reusable conclusions.",
            object!({"task_id":string,"goal":string}),
            vec!["task_id"],
            false,
        ),
        spec(
            "memory_record_result",
            "Record sanitized progress/next steps and source-backed reusable findings/decisions before finishing, where no Stop hook asks for the native-result save; where one asks, that save is the record. Supply the current task revision and a stable result_id: exact completed replay preserves later edits, and repeating an interrupted save with the same result_id finishes it without duplicating what it already wrote. To confirm/correct inspected existing knowledge, preserve type/title/sources and include record_id, expected_revision and its complete checked source_fingerprints. A protected/conflicting learning is held individually while independent progress and valid facts save. verified=true is agent attestation, not human review. Empty learnings is valid.",
            object!({"task_id":string,"result_id":string,"revision":{"type":"integer","minimum":1},"progress":string,"next_steps":strings,"verified":{"type":"boolean"},"capture_decision":{"enum":["selected","no_reusable_knowledge","insufficiently_checked"]},"learnings":{"type":"array","maxItems":3,"items":learning}}),
            vec!["task_id", "result_id", "revision"],
            false,
        ),
    ]
}
