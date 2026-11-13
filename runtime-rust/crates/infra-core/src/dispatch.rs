//! The governed agent audit journal and delegation-capsule contract.
use crate::{
    BrainRepository, MessageInput, Result, RuntimeContext, RuntimeError, Store, TaskResolver,
    TaskService, domain, storage,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
const SECTIONS: &[&str] = &[
    "objective",
    "output format",
    "tool and source guidance",
    "boundaries",
    "decisions and assumptions",
];
pub fn validate_capsule_text(content: &str) -> Result<Vec<String>> {
    if content.trim().is_empty() {
        return Ok(vec!["capsule is empty".into()]);
    }
    let mut problems = Vec::new();
    let length = content.chars().count();
    if length > 8000 {
        problems.push(format!("capsule exceeds 8000 characters ({length})"));
    }
    let lowered = content.to_lowercase();
    for section in SECTIONS {
        if !lowered.contains(section) {
            problems.push(format!("missing mandatory section: {section}"));
        }
    }
    if let Err(error) = domain::guard_shared_text("delegation capsule", &[content.into()]) {
        problems.push(error.to_string());
    }
    Ok(problems)
}
pub fn capsule_report(content: &str) -> Result<Value> {
    let problems = validate_capsule_text(content)?;
    Ok(json!({"valid":problems.is_empty(),"length":content.chars().count(),"problems":problems}))
}
pub struct DispatchService {
    pub context: RuntimeContext,
}
impl DispatchService {
    pub fn new(context: RuntimeContext) -> Self {
        Self { context }
    }
    pub fn task_reference(
        &self,
        store: Option<&mut Store>,
        task: &str,
        provision: bool,
    ) -> Result<String> {
        if self.context.mode == "lightweight" {
            return Err(RuntimeError::policy("Agent messages require governed mode"));
        }
        let id = storage::validate_task_id(task)?;
        let mut resolver = TaskService::new(self.context.clone(), store);
        if provision {
            let _ = resolver.provision(&id);
        }
        if let Some(uuid) = resolver.served_uuid(&id)? {
            return Ok(uuid);
        }
        if let Some(store) = resolver.store.as_deref()
            && let Some(binding) = store.binding(&id)?
        {
            return Ok(binding.task_uuid);
        }
        Ok(id)
    }
    pub fn send(
        &self,
        store: Option<&mut Store>,
        task: &str,
        input: MessageInput,
    ) -> Result<Value> {
        let reference = self.task_reference(store, task, false)?;
        BrainRepository::new(self.context.clone())?.append_message(&reference, input)
    }
    pub fn read(
        &self,
        store: Option<&mut Store>,
        task: &str,
        actor: Option<&str>,
        since: u64,
        kind: Option<&str>,
    ) -> Result<Vec<Value>> {
        let reference = self.task_reference(store, task, false)?;
        BrainRepository::new(self.context.clone())?.read_messages(&reference, actor, since, kind)
    }
    pub fn dispatch(
        &self,
        store: Option<&mut Store>,
        task: &str,
        agent: &str,
        event: &str,
        note: Option<&str>,
        capsule: Option<&str>,
    ) -> Result<Value> {
        let agent = crate::brain::actor(agent, false)?;
        if !["spawn", "complete"].contains(&event) {
            return Err(RuntimeError::invalid(
                "Dispatch event must be spawn or complete",
            ));
        }
        let reference = self.task_reference(store, task, true)?;
        let digest = if let Some(content) = capsule {
            let problems = validate_capsule_text(content)?;
            if !problems.is_empty() && event == "spawn" {
                return Err(RuntimeError::invalid(format!(
                    "Delegation capsule is under-specified: {}",
                    problems.join("; ")
                )));
            }
            Some(crate::domain::hex_lower(Sha256::digest(content.as_bytes())))
        } else {
            None
        };
        let note = note
            .filter(|value| !value.is_empty())
            .map(String::from)
            .unwrap_or_else(|| format!("{event}: {agent}"));
        domain::guard_shared_text("dispatch note", std::slice::from_ref(&note))?;
        let (from_actor, to_actor, message_type) = if event == "spawn" {
            ("main".into(), agent, "dispatch")
        } else {
            (agent, "main".into(), "completion")
        };
        BrainRepository::new(self.context.clone())?.append_message(
            &reference,
            MessageInput {
                from_actor,
                to_actor,
                message_type: message_type.into(),
                body: note.trim().into(),
                refs: Vec::new(),
                capsule_digest: digest,
            },
        )
    }
}

impl DispatchService {
    pub fn execute(&self, operation: &str, request: &Value, input: &[u8]) -> Result<Value> {
        let field = |key: &str| {
            request
                .get(key)
                .or_else(|| request.get(key.replace('_', "-")))
                .and_then(Value::as_str)
        };
        let required = |key: &str| {
            field(key)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| RuntimeError::invalid(format!("Missing {key}")))
        };
        let content = |path: &str| -> Result<String> {
            if path == "-" {
                String::from_utf8(input.to_vec())
                    .map(|text| text.replace("\r\n", "\n").replace('\r', "\n"))
                    .map_err(|_| RuntimeError::invalid("Capsule input must be UTF-8"))
            } else {
                crate::fs::read_text(std::path::Path::new(path))
            }
        };
        match operation {
            "capsule" => capsule_report(&content(field("file").unwrap_or("-"))?),
            "msg-dispatch" => {
                let capsule = field("capsule_file").map(content).transpose()?;
                let db = self
                    .context
                    .roots
                    .state
                    .join("memory-bank/local/context.db");
                let mut store = Store::open(&db, crate::OpenOptions::default())?;
                self.dispatch(
                    Some(&mut store),
                    required("task_id")?,
                    required("agent")?,
                    required("event")?,
                    field("note"),
                    capsule.as_deref(),
                )
            }
            _ => Err(RuntimeError::invalid(format!(
                "Unknown dispatch operation: {operation}"
            ))),
        }
    }
}
