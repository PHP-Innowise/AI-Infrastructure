//! Concrete binding to the production Rust runtime; never imports project Python.
use super::projection::{NativeRequest, NativeResponse, NativeRuntime};
use anyhow::{Result, bail};
use std::{
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

struct CachedStoreSession {
    canon_repo: PathBuf,
    db_path: PathBuf,
    #[cfg(unix)]
    db_dev: u64,
    #[cfg(unix)]
    db_ino: u64,
    #[cfg(not(unix))]
    db_exists: bool,
    runtime_config_stamp: Option<(u64, i128)>,
    context: infra_core::RuntimeContext,
    store: infra_core::Store,
}

fn file_stamp(path: &Path) -> Result<Option<(u64, i128)>> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() || !meta.is_file() {
                bail!("runtime_config_symlink_or_non_file");
            }
            let mtime_ns = meta
                .modified()?
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as i128)
                .unwrap_or(0);
            Ok(Some((meta.len(), mtime_ns)))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

#[cfg(unix)]
fn db_identity(path: &Path) -> Result<Option<(u64, u64)>> {
    use std::os::unix::fs::MetadataExt;
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() || !meta.is_file() {
                bail!("context_db_symlink_or_non_file");
            }
            Ok(Some((meta.dev(), meta.ino())))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

#[cfg(not(unix))]
fn db_identity(path: &Path) -> Result<Option<bool>> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() || !meta.is_file() {
                bail!("context_db_symlink_or_non_file");
            }
            Ok(Some(true))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

pub struct CoreRuntime {
    pub actor: String,
    clock: Option<infra_core::domain::clock::WallClock>,
    session: Option<CachedStoreSession>,
    session_opens: u64,
    session_reuses: u64,
}
impl Default for CoreRuntime {
    fn default() -> Self {
        Self::new("agent")
    }
}
impl CoreRuntime {
    pub fn new(actor: impl Into<String>) -> Self {
        Self {
            actor: actor.into(),
            clock: None,
            session: None,
            session_opens: 0,
            session_reuses: 0,
        }
    }
    /// Explicit trusted evaluation input. Ordinary instances use the system
    /// knowledge clock and never inspect MEMORY_EVAL_CLOCK.
    pub fn with_eval_clock(actor: impl Into<String>, anchor: &str) -> Result<Self> {
        let anchor = infra_core::domain::clock::parse_python_anchor(anchor)?;
        Ok(Self {
            actor: actor.into(),
            clock: Some(infra_core::domain::clock::WallClock::Offset(
                anchor - chrono::Utc::now(),
            )),
            session: None,
            session_opens: 0,
            session_reuses: 0,
        })
    }
    pub fn with_clock(
        actor: impl Into<String>,
        clock: infra_core::domain::clock::WallClock,
    ) -> Self {
        Self {
            actor: actor.into(),
            clock: Some(clock),
            session: None,
            session_opens: 0,
            session_reuses: 0,
        }
    }
    /// Explicitly closes any cached SQLite store connection, returning [`CleanupFailure`]
    /// if the underlying connection fails to close cleanly.
    pub fn close(&mut self) -> Result<()> {
        if let Some(session) = self.session.take()
            && session.store.conn.close().is_err()
        {
            return Err(CleanupFailure.into());
        }
        Ok(())
    }
    pub fn session_opens(&self) -> u64 {
        self.session_opens
    }
    pub fn session_reuses(&self) -> u64 {
        self.session_reuses
    }
}
impl Drop for CoreRuntime {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            let _ = session.store.conn.close();
        }
    }
}
#[derive(Debug)]
pub struct CleanupFailure;
impl std::fmt::Display for CleanupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("native connection cleanup failed")
    }
}
impl std::error::Error for CleanupFailure {}
impl NativeRuntime for CoreRuntime {
    fn close(&mut self) -> Result<()> {
        CoreRuntime::close(self)
    }
    fn refresh_and_render(
        &mut self,
        repository: &Path,
        request: NativeRequest,
    ) -> Result<NativeResponse> {
        let _clock = self.clock.clone().map(infra_core::domain::clock::enter);
        let prompt = infra_core::domain::sanitize_automatic_query(&request.query)?;
        if prompt.is_empty() {
            return Ok(NativeResponse {
                text: String::new(),
                has_evidence: false,
                warnings: false,
                sanitized_empty: true,
            });
        }
        infra_core::domain::guard_shared_text(
            "Task Capsule request",
            std::slice::from_ref(&prompt),
        )?;
        let canon_repo = infra_core::fs::python_realpath(repository)?;
        let db_path = canon_repo.join("memory-bank/local/context.db");
        let config_stamp = file_stamp(&canon_repo.join("project-brain/config/runtime.json"))?;
        let current_db_id = db_identity(&db_path)?;

        let can_reuse = self.session.as_ref().is_some_and(|session| {
            let id_matches = {
                #[cfg(unix)]
                {
                    current_db_id == Some((session.db_dev, session.db_ino))
                }
                #[cfg(not(unix))]
                {
                    current_db_id.is_some() == session.db_exists
                }
            };
            session.canon_repo == canon_repo
                && session.db_path == db_path
                && session.runtime_config_stamp == config_stamp
                && session.context.actor == self.actor
                && id_matches
                && session.store.conn.is_autocommit()
        });

        let mut session = if can_reuse {
            self.session_reuses += 1;
            self.session.take().unwrap()
        } else {
            self.close()?;
            let roots = infra_core::Roots::installed(&canon_repo)?;
            let config = infra_core::RuntimeConfig::load(&roots)?;
            let context =
                infra_core::RuntimeContext::new(roots, self.actor.clone(), config.mode(None)?)?;
            let store = infra_core::Store::open(&db_path, infra_core::OpenOptions::refresh(None))?;
            let post_db_id = db_identity(&db_path)?;
            self.session_opens += 1;
            #[cfg(unix)]
            let (db_dev, db_ino) = post_db_id.unwrap_or((0, 0));
            CachedStoreSession {
                canon_repo,
                db_path,
                #[cfg(unix)]
                db_dev,
                #[cfg(unix)]
                db_ino,
                #[cfg(not(unix))]
                db_exists: post_db_id.is_some(),
                runtime_config_stamp: config_stamp,
                context,
                store,
            }
        };

        let operation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || -> Result<NativeResponse> {
                let capsule = infra_core::retrieve(
                    &mut session.store,
                    &session.context,
                    &infra_core::RetrievalRequest {
                        query: prompt,
                        task_id: Some(request.task_id),
                        limit: request.limit,
                        host: request.host.into(),
                        entry_point: request.entry_point.into(),
                        ephemeral: request.ephemeral,
                        capsule_profile: Some(request.capsule_profile.into()),
                        query_source: "prompt".into(),
                        allow_unprovisioned: request.allow_unprovisioned,
                        automatic_links: true,
                        refresh_index: true,
                        ..Default::default()
                    },
                )?;
                let text =
                    infra_core::capsule::render(&capsule, infra_core::DeliveryProfile::Ordinary)?;
                let evidence = ["semantic", "episodic"]
                    .iter()
                    .any(|k| capsule.0[*k].as_array().is_some_and(|a| !a.is_empty()));
                let warnings = capsule.0["warnings"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty());
                Ok(NativeResponse {
                    text,
                    has_evidence: evidence,
                    warnings,
                    sanitized_empty: false,
                })
            },
        ));
        match operation {
            Ok(Ok(response)) if session.store.conn.is_autocommit() => {
                self.session = Some(session);
                Ok(response)
            }
            Ok(result) => {
                if session.store.conn.close().is_err() {
                    return Err(CleanupFailure.into());
                }
                result
            }
            Err(payload) => {
                if session.store.conn.close().is_err() {
                    return Err(CleanupFailure.into());
                }
                std::panic::resume_unwind(payload)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_minimal_repo(dir: &Path) {
        std::fs::create_dir_all(dir.join("project-brain/config")).unwrap();
        std::fs::create_dir_all(dir.join("memory-bank/chunks")).unwrap();
        std::fs::write(
            dir.join("project-brain/config/runtime.json"),
            r#"{"schema_version":1,"mode":"governed"}"#,
        )
        .unwrap();
        std::fs::write(dir.join("memory-bank/INDEX.md"), "# Index\n").unwrap();
    }

    #[test]
    fn core_runtime_reuses_healthy_store_session_and_invalidates_on_config_or_repo_drift() {
        let base = std::env::temp_dir().join(format!(
            "infra-pilots-core-runtime-{}",
            uuid::Uuid::new_v4()
        ));
        let repo_a = base.join("repo-a");
        let repo_b = base.join("repo-b");
        init_minimal_repo(&repo_a);
        init_minimal_repo(&repo_b);

        let mut rt = CoreRuntime::new("pilot-test");
        let req = || NativeRequest {
            query: "auth flow".into(),
            task_id: "TASK-001".into(),
            limit: 3,
            ephemeral: true,
            capsule_profile: "full",
            host: "cli",
            entry_point: "refresh",
            allow_unprovisioned: true,
        };

        let r1 = rt.refresh_and_render(&repo_a, req()).unwrap();
        assert!(!r1.sanitized_empty);
        assert_eq!(rt.session_opens(), 1);
        assert_eq!(rt.session_reuses(), 0);

        // Repeated query on same repository with unchanged runtime.json & context.db reuses connection.
        let r2 = rt.refresh_and_render(&repo_a, req()).unwrap();
        assert!(!r2.sanitized_empty);
        assert_eq!(rt.session_opens(), 1);
        assert_eq!(rt.session_reuses(), 1);

        // Mutating runtime.json invalidates cached context/session and triggers clean reopen.
        let mut cfg_bytes = br#"{"schema_version":1,"mode":"governed","note":"updated"}"#.to_vec();
        cfg_bytes.push(b'\n');
        std::fs::write(repo_a.join("project-brain/config/runtime.json"), cfg_bytes).unwrap();
        let _ = rt.refresh_and_render(&repo_a, req()).unwrap();
        assert_eq!(rt.session_opens(), 2);
        assert_eq!(rt.session_reuses(), 1);

        // Switching to a second repository closes repo-a connection cleanly and opens repo-b.
        let _ = rt.refresh_and_render(&repo_b, req()).unwrap();
        assert_eq!(rt.session_opens(), 3);
        assert_eq!(rt.session_reuses(), 1);

        // Explicit close cleans up cached connection and forces reopen on subsequent query.
        rt.close().unwrap();
        assert!(rt.session.is_none());
        let _ = rt.refresh_and_render(&repo_b, req()).unwrap();
        assert_eq!(rt.session_opens(), 4);

        let _ = std::fs::remove_dir_all(&base);
    }
}
