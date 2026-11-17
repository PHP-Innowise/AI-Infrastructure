use crate::{
    HarnessError, Result, fleet::Blackboard, knowledge::KnowledgeDomain,
    sessions::BlackboardFactory,
};
use infra_core::{
    BrainRepository, DispatchService, OpenOptions, Roots, RuntimeContext, StartTask, Store,
    UpdateRecord,
};
use serde_json::Value;
use std::{path::PathBuf, sync::Arc};
pub struct CoreBlackboards {
    pub knowledge: Arc<KnowledgeDomain>,
}
pub struct CoreBlackboard {
    pub context: RuntimeContext,
    pub available: bool,
}
impl BlackboardFactory for CoreBlackboards {
    fn for_session(&self, session: &Value) -> Result<Arc<dyn Blackboard>> {
        let _source_frame = infra_core::git::source_frame_scope();
        let root = PathBuf::from(
            session["project_path"]
                .as_str()
                .ok_or_else(|| HarnessError::invalid("Session workspace is missing."))?,
        );
        let project = session["project_id"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Session project is missing."))?;
        let context = match self.knowledge.context(project, Some(&root)) {
            Ok(context) => context,
            Err(error) if error.status == 404 => {
                RuntimeContext::new(Roots::installed(&root)?, "harness", "governed")?
            }
            Err(error) => return Err(error),
        };
        let available = context
            .roots
            .state
            .join("project-brain/config/runtime.json")
            .is_file();
        Ok(Arc::new(CoreBlackboard { context, available }))
    }
}
impl CoreBlackboard {
    fn store(&self) -> Result<Store> {
        Ok(Store::open(
            self.context
                .roots
                .state
                .join("memory-bank/local/context.db"),
            OpenOptions::default(),
        )?)
    }
}
impl Blackboard for CoreBlackboard {
    fn available(&self) -> bool {
        self.available
    }
    fn ensure_task(&self, task: &str, title: &str) -> Result<()> {
        let _source_frame = infra_core::git::source_frame_scope();
        // A project without the optional governed runtime has no audit store to mutate.
        if !self.available {
            return Ok(());
        }
        let mut store = self.store()?;
        if store.working(task)?.is_none() {
            store.start(StartTask {
                task_id: task.into(),
                goal: title.into(),
                files: vec![],
                sources: vec![],
            })?;
        }
        let mut service =
            infra_core::serving::TaskService::new(self.context.clone(), Some(&mut store));
        service.checkpoint(task, Some(title))?;
        Ok(())
    }
    fn dispatch_spawn(&self, task: &str, lens: &str, capsule: &str) -> Result<()> {
        let _source_frame = infra_core::git::source_frame_scope();
        if !self.available {
            return Ok(());
        }
        let problems = infra_core::dispatch::validate_capsule_text(capsule)?;
        if !problems.is_empty() {
            return Err(HarnessError::invalid(format!(
                "Delegation capsule is under-specified: {}",
                problems.join("; ")
            )));
        }
        let mut store = self.store()?;
        DispatchService::new(self.context.clone()).dispatch(
            Some(&mut store),
            task,
            lens,
            "spawn",
            None,
            Some(capsule),
        )?;
        Ok(())
    }
    fn dispatch_complete(&self, task: &str, lens: &str, message: &str) -> Result<()> {
        let _source_frame = infra_core::git::source_frame_scope();
        if !self.available {
            return Ok(());
        }
        let mut store = self.store()?;
        let note = message
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(160)
            .collect::<String>();
        DispatchService::new(self.context.clone()).dispatch(
            Some(&mut store),
            task,
            lens,
            "complete",
            Some(&note),
            None,
        )?;
        Ok(())
    }
    fn progress(&self, task: &str, message: &str) -> Result<()> {
        let _source_frame = infra_core::git::source_frame_scope();
        if !self.available {
            return Ok(());
        }
        let brain = BrainRepository::new(self.context.clone())?;
        // Fleet lenses report at once: a write that lost the race to another
        // reads the task again rather than dropping its progress.
        let mut attempts = 0;
        loop {
            let record = brain.get_task(task)?;
            let written = brain.update(
                record["id"]
                    .as_str()
                    .ok_or_else(|| HarnessError::invalid("Task identity is missing."))?,
                UpdateRecord {
                    expected_revision: record["revision"].as_u64(),
                    progress: Some(message.chars().take(500).collect()),
                    actor: self.context.actor.clone(),
                    ..UpdateRecord::default()
                },
            );
            match written {
                Err(error) if error.kind == infra_core::ErrorKind::Conflict && attempts < 8 => {
                    attempts += 1;
                    std::thread::sleep(std::time::Duration::from_millis(5 * attempts));
                }
                written => {
                    written?;
                    return Ok(());
                }
            }
        }
    }
}

#[cfg(test)]
mod progress_tests {
    use super::*;

    #[test]
    fn progress_survives_concurrent_lenses() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project");
        std::fs::create_dir_all(root.join("project-brain/config")).unwrap();
        std::fs::write(
            root.join("project-brain/config/runtime.json"),
            r#"{"mode":"governed"}"#,
        )
        .unwrap();
        let schemas = root.join("project-brain/schemas");
        std::fs::create_dir_all(&schemas).unwrap();
        for entry in std::fs::read_dir(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../PHP Core/project-brain/schemas"),
        )
        .unwrap()
        {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), schemas.join(entry.file_name())).unwrap();
        }
        let context =
            RuntimeContext::new(Roots::installed(&root).unwrap(), "local", "governed").unwrap();
        BrainRepository::new(context.clone())
            .unwrap()
            .create(infra_core::CreateRecord {
                external_id: "TASK-FLEET".into(),
                title: "Review the change".into(),
                ..infra_core::CreateRecord::default()
            })
            .unwrap();
        let board = Arc::new(CoreBlackboard {
            context: context.clone(),
            available: true,
        });
        let lenses = (0..4)
            .map(|lens| {
                let board = board.clone();
                std::thread::spawn(move || {
                    (0..2)
                        .map(|turn| {
                            board.progress("TASK-FLEET", &format!("lens {lens} turn {turn}"))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        for lens in lenses {
            for report in lens.join().unwrap() {
                report.unwrap();
            }
        }
        let task = BrainRepository::new(context)
            .unwrap()
            .get_task("TASK-FLEET")
            .unwrap();
        // Every report landed: the task moved past its first revision by each.
        assert!(task["revision"].as_u64().unwrap() >= 9, "{task}");
    }
}
