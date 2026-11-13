//! Authoritative task admission/provisioning for result, native and MCP callers.
use crate::{
    BrainRepository, CreateRecord, ErrorKind, Result, RuntimeConfig, RuntimeContext, RuntimeError,
    Store, domain::*, git::GitFrame,
};
use serde_json::{Value, json};

pub trait TaskResolver {
    fn provision(&mut self, external_id: &str) -> Result<Value>;
    fn served_uuid(&self, external_id: &str) -> Result<Option<String>>;
}
pub struct TaskService<'a> {
    pub context: RuntimeContext,
    pub store: Option<&'a mut Store>,
}
impl<'a> TaskService<'a> {
    pub fn new(context: RuntimeContext, store: Option<&'a mut Store>) -> Self {
        Self { context, store }
    }
    pub fn brain(&self) -> Result<BrainRepository> {
        BrainRepository::new(self.context.clone())
    }
    pub fn authorized(&self, record: Value, active: bool) -> Result<Value> {
        let config = RuntimeConfig::load(&self.context.roots)?;
        if !["public", "team"].contains(&record["privacy"].as_str().unwrap_or(""))
            || !config
                .strings("allowed_privacy")?
                .iter()
                .any(|value| record["privacy"].as_str() == Some(value.as_str()))
        {
            return Err(RuntimeError::coded(
                ErrorKind::Unauthorized,
                "memory-result-v1:task_privacy",
                "Task is outside the allowed memory privacy scope",
            ));
        }
        if !record["authorized_owners"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|owner| owner.as_str() == Some(self.context.actor.as_str()))
        {
            return Err(RuntimeError::coded(
                ErrorKind::Unauthorized,
                "memory-result-v1:not_authorized",
                "The configured actor is not authorized for this task",
            ));
        }
        if active && matches!(record["status"].as_str(), Some("completed" | "cancelled")) {
            return Err(RuntimeError::coded(
                ErrorKind::Conflict,
                "memory-result-v1:task_closed",
                "Use an active task to record new memory",
            ));
        }
        Ok(record)
    }
    pub fn task(&self, id: &str, active: bool) -> Result<Value> {
        let brain = self.brain()?;
        match brain.get_task(id) {
            Ok(record) => self.authorized(record, active),
            Err(error) if error.kind == ErrorKind::Conflict => {
                let uuid = self.served_uuid(id)?.ok_or(error)?;
                let task = brain.get_task(&uuid)?;
                if task["external_id"] != id {
                    return Err(RuntimeError::conflict(
                        "Served task does not match its alias",
                    ));
                }
                self.authorized(task, active)
            }
            Err(error) => Err(error),
        }
    }
    pub fn checkpoint(&mut self, id: &str, goal: Option<&str>) -> Result<Value> {
        let brain = self.brain()?;
        let _guard = brain.lock()?;
        let task = match brain.get_task(id) {
            Ok(task) => self.authorized(task, true)?,
            Err(error) if error.kind == ErrorKind::NotFound => {
                let goal = goal.ok_or_else(|| RuntimeError::invalid("A new task needs a goal"))?;
                guard_shared_text("Task goal", &[goal.into()])?;
                brain.create_task(id, goal, vec![], vec![], "team")?
            }
            Err(error) => return Err(error),
        };
        let checkpoint = if let Some(store) = self.store.as_deref_mut() {
            if store.binding(id)?.is_none() {
                store.bind(
                    id,
                    task["id"].as_str().unwrap_or(""),
                    task["revision"].as_u64().unwrap_or(1),
                )?;
            }
            crate::checkpoint::flush(store, &self.context, id, 20)?
        } else {
            None
        };
        let current = brain.get_task(task["id"].as_str().unwrap_or(""))?;
        Ok(
            json!({"task_id":id,"task_uuid":current["id"],"revision":current["revision"],"checkpoint":checkpoint}),
        )
    }
}
impl TaskResolver for TaskService<'_> {
    fn provision(&mut self, id: &str) -> Result<Value> {
        if let Some(store) = self.store.as_deref_mut() {
            crate::checkpoint::ensure(store, &self.context, id)?;
            let uuid = crate::checkpoint::served_uuid(Some(store), &self.context, id)?
                .ok_or_else(|| RuntimeError::not_found("Working task unavailable"))?;
            return self
                .brain()?
                .get_task(&uuid)
                .and_then(|task| self.authorized(task, true));
        }
        let brain = self.brain()?;
        let _guard = brain.lock()?;
        match brain.get_task(id) {
            Ok(record) => self.authorized(record, true),
            Err(error) if error.kind == ErrorKind::NotFound => {
                let goal = crate::storage::derive_goal(id);
                let task = brain.create(CreateRecord {
                    external_id: id.into(),
                    title: goal.clone(),
                    goal: Some(goal),
                    owner: self.context.actor.clone(),
                    ..CreateRecord::default()
                })?;
                if let Some(store) = self.store.as_deref_mut() {
                    store.bind(
                        id,
                        task["id"].as_str().unwrap_or(""),
                        task["revision"].as_u64().unwrap_or(1),
                    )?;
                }
                Ok(task)
            }
            Err(error) => Err(error),
        }
    }
    fn served_uuid(&self, id: &str) -> Result<Option<String>> {
        crate::checkpoint::served_uuid(self.store.as_deref(), &self.context, id)
    }
}
pub fn checkout_identity(context: &RuntimeContext) -> Result<String> {
    let frame = GitFrame::discover(&context.roots.project)?;
    let value = match frame {
        Some(frame) => json!([context.roots.project, frame.admin, frame.common_dir()?]),
        None => json!([context.roots.project]),
    };
    Ok(crate::query::sha256(
        json::python_default(&value, false, false, true)?.as_bytes(),
    ))
}
