use crate::{Result, RuntimeError, fs, git::GitFrame};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Roots {
    pub tooling: PathBuf,
    pub project: PathBuf,
    pub state: PathBuf,
    pub shared: bool,
}

impl Roots {
    pub fn installed(repository: impl Into<PathBuf>) -> Result<Self> {
        let root = fs::python_realpath(&repository.into())?;
        Ok(Self {
            tooling: root.clone(),
            project: root.clone(),
            state: root,
            shared: false,
        })
    }

    pub fn attached(&self) -> bool {
        self.project != self.state
    }

    pub fn discover(repository: &Path, tooling: &Path) -> Result<Self> {
        let repository = fs::python_realpath(repository)?;
        let tooling = fs::python_realpath(tooling)?;
        let home = std::env::var_os("ACCELERATOR_HOME").map(PathBuf::from);
        let names_copy = match home {
            Some(home) => fs::python_realpath(&home)? == tooling,
            None => false,
        };
        if names_copy {
            let state = std::env::var_os("ACCELERATOR_STATE_DIR")
                .map(PathBuf::from)
                .ok_or_else(|| RuntimeError::invalid("ACCELERATOR_STATE_DIR must be set"))?;
            let project = std::env::var_os("ACCELERATOR_PROJECT_DIR")
                .map(PathBuf::from)
                .ok_or_else(|| RuntimeError::invalid("ACCELERATOR_PROJECT_DIR must be set"))?;
            if !state.is_absolute() || !project.is_absolute() || !project.is_dir() {
                return Err(RuntimeError::invalid(
                    "Attached state and project must be absolute; project must exist",
                ));
            }
            for entry in std::iter::once(state.clone()).chain(
                [
                    "project-brain",
                    "memory-bank",
                    "launch",
                    "accelerator-attach.json",
                ]
                .into_iter()
                .map(|name| state.join(name)),
            ) {
                match std::fs::symlink_metadata(&entry) {
                    Ok(metadata) if fs::linked_at(&entry, &metadata)? => {
                        return Err(RuntimeError::policy(
                            "Attached state must not be reached through a link",
                        ));
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
            let state = fs::python_realpath(&state)?;
            let project = fs::python_realpath(&project)?;
            if state == project || state.starts_with(&project) {
                return Err(RuntimeError::policy(
                    "Attached state must lie outside its project",
                ));
            }
            if state == repository {
                for name in [
                    "project-brain",
                    "memory-bank",
                    "launch",
                    "accelerator-attach.json",
                ] {
                    fs::admit_descendant(&state, &state.join(name))?;
                }
                return Ok(Self {
                    tooling: tooling.clone(),
                    project,
                    state,
                    shared: tooling == repository,
                });
            }
        }
        if repository == tooling {
            let start = std::env::var_os("CLAUDE_PROJECT_DIR")
                .map(PathBuf::from)
                .unwrap_or(std::env::current_dir()?);
            if let (Some(served), Some(installation)) =
                (GitFrame::discover(&start)?, GitFrame::discover(&tooling)?)
                && served.top != installation.top
                && served.common_dir()? == installation.common_dir()?
            {
                let project = fs::python_realpath_spelling(&start, served.top)?;
                return Ok(Self {
                    tooling,
                    project,
                    state: repository,
                    shared: true,
                });
            }
        }
        Self::installed(repository)
    }

    pub fn resolve(&self, key: &str) -> PathBuf {
        let relative = key.split('#').next().unwrap_or(key);
        let path = Path::new(relative);
        if path.is_absolute() {
            return path.to_path_buf();
        }
        if self.attached() && is_state_key(relative) {
            return self.state.join(path);
        }
        if self.shared && accelerator_owned_source(relative) {
            let candidate = self.project.join(path);
            if std::fs::symlink_metadata(&candidate)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
            {
                return self.tooling.join(path);
            }
            return candidate;
        }
        self.project.join(path)
    }

    pub fn contains(&self, path: &Path) -> Result<bool> {
        let path = fs::python_realpath(path)?;
        for base in [&self.project, &self.state, &self.tooling] {
            let base = fs::python_realpath(base)?;
            if path.starts_with(base) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn key_for(&self, base: &Path, path: &Path) -> Result<String> {
        if self.attached() && base == self.tooling {
            let relative = path
                .strip_prefix(base)
                .map_err(|_| RuntimeError::policy("Source escapes root"))?;
            let relative = relative
                .to_str()
                .ok_or_else(|| RuntimeError::invalid("Source path is not UTF-8"))?
                .replace('\\', "/");
            if self.shared && is_state_key(&relative) {
                return Ok(relative);
            }
            return path
                .to_str()
                .map(|s| s.replace('\\', "/"))
                .ok_or_else(|| RuntimeError::invalid("Source path is not UTF-8"));
        }
        let relative = path
            .strip_prefix(base)
            .map_err(|_| RuntimeError::policy("Source escapes root"))?;
        relative
            .to_str()
            .map(|s| s.replace('\\', "/"))
            .ok_or_else(|| RuntimeError::invalid("Source path is not UTF-8"))
    }

    pub fn brain_root(&self) -> PathBuf {
        self.state.join("project-brain")
    }

    pub fn ensure_attached_state(&self) -> Result<Vec<String>> {
        if !self.attached() || self.shared {
            return Ok(vec![]);
        }
        let mut files = crate::bank::publication::PublicationFiles::open_private_root(&self.state)?;
        self.ensure_attached_state_in(&mut files)
    }

    /// Seed using the same retained state custody acquired by the launcher.
    pub fn ensure_attached_state_in(
        &self,
        files: &mut crate::bank::publication::PublicationFiles,
    ) -> Result<Vec<String>> {
        if !self.attached() || self.shared {
            return Ok(vec![]);
        }
        let expected_state = fs::absolute(&self.state)?;
        if files.root_path() != expected_state.as_path() {
            return Err(RuntimeError::policy(
                "Attached state custody names another root",
            ));
        }
        let mut source = crate::bank::publication::PublicationFiles::new(&self.tooling)?;
        let mut seeds = Vec::new();
        for (target, origin) in [
            (
                "project-brain/config/runtime.json",
                "project-brain/config/runtime.json",
            ),
            (
                "project-brain/config/telemetry.json",
                "project-brain/config/telemetry.json",
            ),
            (
                "project-brain/config/providers.json",
                "project-brain/config/providers.json",
            ),
            (
                "project-brain/indexes/active.json",
                "project-brain/.install/active.json",
            ),
            (
                "project-brain/indexes/archive.json",
                "project-brain/.install/archive.json",
            ),
            ("memory-bank/INDEX.md", "memory-bank/.install/INDEX.md"),
        ] {
            if let Some(bytes) = source.read(origin, 8 * 1024 * 1024)? {
                seeds.push((target, bytes));
            }
        }
        self.secure_attached_state_in(files)?;
        let _guard = files.acquire_private_state_lock()?;
        crate::bank::retirement::recover_pending_in(files)?;
        let mut created = Vec::new();
        for directory in [
            "project-brain/archive",
            "project-brain/control/handoffs",
            "project-brain/control/messages",
            "project-brain/control/promotions",
            "project-brain/control/retrieval-manifests",
            "project-brain/dynamic/bugs",
            "project-brain/dynamic/decisions",
            "project-brain/dynamic/events",
            "project-brain/dynamic/findings",
            "project-brain/dynamic/incidents",
            "project-brain/dynamic/tasks",
            "project-brain/indexes",
            "project-brain/local",
            "memory-bank/chunks",
            "memory-bank/local",
        ] {
            if files.secure_private_directory(directory, true)? {
                created.push(format!("{directory}/"));
            }
        }
        source.check()?;
        for (target, bytes) in seeds {
            if files.read(target, 8 * 1024 * 1024)?.is_none() {
                files.write_expected_mode(target, &bytes, None, 0o600, 8 * 1024 * 1024)?;
                created.push(target.into());
            }
            files.secure_private_file(target)?;
        }
        self.secure_attached_state_in(files)?;
        files.check()?;
        Ok(created)
    }

    pub fn secure_attached_state(&self) -> Result<()> {
        if !self.attached() || self.shared {
            return Ok(());
        }
        let mut files = crate::bank::publication::PublicationFiles::open_private_root(&self.state)?;
        self.secure_attached_state_in(&mut files)
    }

    pub fn secure_attached_state_in(
        &self,
        files: &mut crate::bank::publication::PublicationFiles,
    ) -> Result<()> {
        if !self.attached() || self.shared {
            return Ok(());
        }
        let expected_state = fs::absolute(&self.state)?;
        if files.root_path() != expected_state.as_path() {
            return Err(RuntimeError::policy(
                "Attached state custody names another root",
            ));
        }
        files.secure_private_directory("", false)?;
        for directory in ["project-brain", "memory-bank", "launch"] {
            files.secure_private_tree(directory)?;
        }
        if files
            .read("accelerator-attach.json", 1024 * 1024)?
            .is_some()
        {
            files.secure_private_file("accelerator-attach.json")?;
        }
        files.check()
    }
}

pub fn is_state_key(key: &str) -> bool {
    key.starts_with("project-brain/") || key.starts_with("memory-bank/")
}

pub fn accelerator_owned_source(key: &str) -> bool {
    key == "AGENTS.md"
        || [
            ".agents/skills/",
            ".claude/skills/",
            ".cursor/skills/",
            ".codex/skills/",
            "memory-bank/chunks/",
            "specs/",
            "tasks/",
        ]
        .iter()
        .any(|prefix| key.starts_with(prefix))
}

#[derive(Clone, Debug)]
pub struct RuntimeContext {
    pub roots: Roots,
    pub actor: String,
    pub mode: String,
}

impl RuntimeContext {
    pub fn new(roots: Roots, actor: impl Into<String>, mode: impl Into<String>) -> Result<Self> {
        let actor = actor.into();
        let mode = mode.into();
        if actor.trim().is_empty() {
            return Err(RuntimeError::invalid("Owner must not be empty"));
        }
        if !["governed", "lightweight"].contains(&mode.as_str()) {
            return Err(RuntimeError::invalid(
                "Mode must be governed or lightweight",
            ));
        }
        Ok(Self { roots, actor, mode })
    }
}
