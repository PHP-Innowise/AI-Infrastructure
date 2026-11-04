//! Rooted storage for the review workspace's feedback.json only.
use crate::{Result, RuntimeError, common};
use infra_core::bank::publication::{FileImageIdentity, PublicationFiles};
use serde_json::Value;
use std::path::Path;

const NAME: &str = "feedback.json";
pub(super) const LIMIT: usize = 8 * 1024 * 1024;

pub(super) fn json_content_type(value: &str) -> bool {
    value
        .split(';')
        .next()
        .is_some_and(|value| value.trim() == "application/json")
}

struct Snapshot {
    bytes: Option<Vec<u8>>,
    image: Option<FileImageIdentity>,
    mode: Option<u32>,
}

pub(super) struct FeedbackStore {
    files: PublicationFiles,
    verifier: PublicationFiles,
}
impl FeedbackStore {
    pub(super) fn open(workspace: &Path) -> Result<Self> {
        let files = PublicationFiles::new(workspace)?;
        let verifier = PublicationFiles::new(workspace)?;
        if files.root_identity() != verifier.root_identity() {
            return Err(RuntimeError::conflict(
                "Feedback workspace changed during open",
            ));
        }
        Ok(Self { files, verifier })
    }

    fn snapshot(&mut self) -> Result<Snapshot> {
        self.files.check()?;
        self.verifier.check()?;
        let image = self.files.image_identity(NAME)?;
        let bytes = self.files.read(NAME, LIMIT)?;
        let mode = self.files.read_mode(NAME)?;
        if self.files.image_identity(NAME)? != image
            || self.verifier.image_identity(NAME)? != image
            || bytes.is_some() != image.is_some()
            || mode.is_some() != image.is_some()
        {
            return Err(RuntimeError::conflict(
                "Feedback image changed during capture",
            ));
        }
        self.files.check()?;
        self.verifier.check()?;
        Ok(Snapshot { bytes, image, mode })
    }

    pub(super) fn read(&mut self) -> Result<Vec<u8>> {
        Ok(self.snapshot()?.bytes.unwrap_or_else(|| b"{}".to_vec()))
    }

    pub(super) fn save(&mut self, body: &[u8]) -> Result<()> {
        if body.len() > LIMIT {
            return Err(RuntimeError::invalid("Feedback exceeds limit"));
        }
        let value: Value = serde_json::from_slice(body)?;
        self.publish(&value, || Ok(()))
    }

    // The observer runs at the actual native prepared-publication boundary.
    // It permits deterministic real-file replacement regressions without a
    // separate filesystem or a simulated publication implementation.
    fn publish<F>(&mut self, value: &Value, before_commit: F) -> Result<()>
    where
        F: FnOnce() -> Result<()>,
    {
        if !value.is_object() || value.get("reviews").is_none() {
            return Err(RuntimeError::invalid(
                "Expected JSON object with 'reviews' key",
            ));
        }
        let content = (common::pretty(value)? + "\n").into_bytes();
        if content.len() > LIMIT {
            return Err(RuntimeError::invalid("Feedback exceeds limit"));
        }
        let snapshot = self.snapshot()?;
        let expected_image = snapshot.image;
        let expected_root = self.files.root_identity();
        // Windows exposes the writable/readonly projection rather than Unix bits.
        let mode = snapshot
            .mode
            .unwrap_or(if cfg!(unix) { 0o600 } else { 0o644 });
        let verifier = &mut self.verifier;
        self.files.write_receipted(
            NAME,
            &content,
            snapshot.bytes.as_deref(),
            snapshot.mode,
            mode,
            LIMIT,
            move |_| {
                before_commit()?;
                verifier.check()?;
                if verifier.root_identity() != expected_root
                    || verifier.image_identity(NAME)? != expected_image
                {
                    return Err(RuntimeError::conflict(
                        "Feedback image changed before publication",
                    ));
                }
                Ok(())
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{fs, path::PathBuf};

    const ORIGINAL: &[u8] = b"{\"reviews\":[{\"run_id\":\"eval-1\",\"feedback\":\"keep me\"}],\"status\":\"in_progress\"}\n";

    fn entries(root: &Path) -> Result<Vec<String>> {
        let mut names = fs::read_dir(root)?
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect::<std::io::Result<Vec<_>>>()?;
        names.sort();
        Ok(names)
    }

    fn make_workspace() -> Result<(tempfile::TempDir, PathBuf)> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("workspace");
        fs::create_dir(&root)?;
        let root = infra_core::fs::canonicalize(root)?;
        Ok((temp, root))
    }

    #[test]
    fn missing_empty_and_known_feedback_keep_the_existing_wire_layout() -> Result<()> {
        let (_temp, root) = make_workspace()?;
        let mut store = FeedbackStore::open(&root)?;
        assert_eq!(store.read()?, b"{}");
        assert!(!root.join(NAME).exists(), "GET created feedback");
        store.save(br#"{"reviews":[]}"#)?;
        assert_eq!(
            PublicationFiles::new(&root)?.read_mode(NAME)?,
            Some(if cfg!(unix) { 0o600 } else { 0o644 }),
            "first-save mode must match the platform's observed projection",
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&store.read()?)?,
            json!({"reviews":[]})
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(root.join(NAME))?.permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_file(root.join(NAME))?;
        fs::write(root.join(NAME), b"")?;
        assert_eq!(store.read()?, b"", "empty ordinary file semantics changed");
        let payload = json!({
            "reviews": [
                {"run_id":"eval-1","feedback":"A\nB","timestamp":"2026-11-02T12:00:00Z"},
                {"run_id":"eval-2","feedback":"","timestamp":"2026-11-02T12:00:00Z"}
            ],
            "status":"complete",
            "operator_note":"additional metadata remains intact"
        });
        store.save(&serde_json::to_vec(&payload)?)?;
        assert_eq!(store.read()?, (common::pretty(&payload)? + "\n").as_bytes());
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(root.join(NAME))?)?,
            payload
        );
        assert_eq!(entries(&root)?, vec![NAME]);
        Ok(())
    }

    #[test]
    fn invalid_payload_headers_and_bounds_do_not_modify_feedback() -> Result<()> {
        let (_temp, root) = make_workspace()?;
        fs::write(root.join(NAME), ORIGINAL)?;
        let mut store = FeedbackStore::open(&root)?;
        for payload in [
            b"null".as_slice(),
            b"[]",
            b"{}",
            b"{\"status\":\"complete\"}",
            b"not-json",
        ] {
            assert!(
                store.save(payload).is_err(),
                "accepted invalid request: {payload:?}"
            );
            assert_eq!(fs::read(root.join(NAME))?, ORIGINAL);
        }
        assert!(store.save(&vec![b' '; LIMIT + 1]).is_err());
        assert_eq!(fs::read(root.join(NAME))?, ORIGINAL);
        assert!(json_content_type("application/json"));
        assert!(json_content_type("application/json; charset=utf-8"));
        assert!(!json_content_type(""));
        assert!(!json_content_type("text/plain"));
        assert!(!json_content_type("application/jsonp"));
        assert!(!json_content_type("text/plain; application/json"));
        assert_eq!(entries(&root)?, vec![NAME]);
        Ok(())
    }

    #[test]
    fn oversized_existing_feedback_is_refused_without_publication() -> Result<()> {
        let (_temp, root) = make_workspace()?;
        fs::write(root.join(NAME), vec![b'x'; LIMIT])?;
        let mut store = FeedbackStore::open(&root)?;
        assert_eq!(store.read()?.len(), LIMIT, "inclusive read limit changed");
        fs::write(root.join(NAME), vec![b'x'; LIMIT + 1])?;
        assert!(store.read().is_err());
        assert!(store.save(br#"{"reviews":[]}"#).is_err());
        assert_eq!(fs::metadata(root.join(NAME))?.len(), (LIMIT + 1) as u64);
        assert_eq!(entries(&root)?, vec![NAME]);
        Ok(())
    }

    #[test]
    fn hardlinked_feedback_never_reads_or_overwrites_an_outside_file() -> Result<()> {
        let (temp, root) = make_workspace()?;
        let outside = temp.path().join("outside.json");
        fs::write(&outside, ORIGINAL)?;
        fs::hard_link(&outside, root.join(NAME))?;
        let mut store = FeedbackStore::open(&root)?;
        assert!(store.read().is_err());
        assert!(
            store
                .save(br#"{"reviews":[],"status":"complete"}"#)
                .is_err()
        );
        assert_eq!(fs::read(&outside)?, ORIGINAL);
        assert_eq!(fs::read(root.join(NAME))?, ORIGINAL);
        assert_eq!(entries(&root)?, vec![NAME]);
        Ok(())
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn symlinked_feedback_never_reads_or_overwrites_an_outside_file() -> Result<()> {
        let (temp, root) = make_workspace()?;
        let outside = temp.path().join("outside.json");
        fs::write(&outside, ORIGINAL)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, root.join(NAME))?;
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&outside, root.join(NAME))?;
        let mut store = FeedbackStore::open(&root)?;
        assert!(store.read().is_err());
        assert!(
            store
                .save(br#"{"reviews":[],"status":"complete"}"#)
                .is_err()
        );
        assert_eq!(fs::read(&outside)?, ORIGINAL);
        assert!(
            fs::symlink_metadata(root.join(NAME))?
                .file_type()
                .is_symlink()
        );
        assert_eq!(entries(&root)?, vec![NAME]);
        Ok(())
    }

    #[test]
    fn precommit_byte_change_is_retained_and_refuses_the_stale_write() -> Result<()> {
        let (_temp, root) = make_workspace()?;
        let path = root.join(NAME);
        fs::write(&path, ORIGINAL)?;
        let mut store = FeedbackStore::open(&root)?;
        let changed = b"{\"reviews\":[],\"external\":\"changed after capture\"}\n";
        let result = store.publish(&json!({"reviews":[],"status":"complete"}), || {
            fs::write(&path, changed)?;
            Ok(())
        });
        assert_eq!(
            result.expect_err("stale image published").kind,
            infra_core::ErrorKind::Conflict
        );
        assert_eq!(fs::read(&path)?, changed);
        assert_eq!(
            entries(&root)?,
            vec![NAME],
            "publication leaked a temporary file"
        );
        Ok(())
    }

    #[test]
    fn precommit_same_bytes_and_mode_replacement_is_an_identity_conflict() -> Result<()> {
        let (_temp, root) = make_workspace()?;
        let path = root.join(NAME);
        let replacement = root.join("replacement.json");
        fs::write(&path, ORIGINAL)?;
        let permissions = fs::metadata(&path)?.permissions();
        let mut store = FeedbackStore::open(&root)?;
        let result = store.publish(&json!({"reviews":[],"status":"complete"}), || {
            fs::write(&replacement, ORIGINAL)?;
            fs::set_permissions(&replacement, permissions)?;
            fs::rename(&replacement, &path)?;
            Ok(())
        });
        assert_eq!(
            result.expect_err("same-byte replacement accepted").kind,
            infra_core::ErrorKind::Conflict
        );
        assert_eq!(fs::read(&path)?, ORIGINAL);
        assert_eq!(
            entries(&root)?,
            vec![NAME],
            "publication leaked a temporary file"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn mode_is_preserved_and_precommit_mode_changes_refuse_publication() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let (_temp, root) = make_workspace()?;
        let path = root.join(NAME);
        fs::write(&path, ORIGINAL)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640))?;
        let mut store = FeedbackStore::open(&root)?;
        store.save(br#"{"reviews":[],"status":"in_progress"}"#)?;
        assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o640);
        let before = fs::read(&path)?;
        let result = store.publish(&json!({"reviews":[],"status":"complete"}), || {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
            Ok(())
        });
        assert_eq!(
            result.expect_err("changed mode accepted").kind,
            infra_core::ErrorKind::Conflict
        );
        assert_eq!(fs::read(&path)?, before);
        assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o600);
        assert_eq!(
            entries(&root)?,
            vec![NAME],
            "publication leaked a temporary file"
        );
        Ok(())
    }

    #[test]
    fn previous_iteration_feedback_uses_the_same_rooted_read() -> Result<()> {
        let (temp, root) = make_workspace()?;
        fs::write(
            root.join(NAME),
            br#"{"reviews":[{"run_id":"previous-run","feedback":"retained previous note"}]}"#,
        )?;
        assert_eq!(
            super::super::load_previous_iteration(&root)?["previous-run"]["feedback"],
            "retained previous note"
        );
        fs::remove_file(root.join(NAME))?;
        let outside = temp.path().join("outside.json");
        fs::write(&outside, ORIGINAL)?;
        fs::hard_link(&outside, root.join(NAME))?;
        assert!(super::super::load_previous_iteration(&root).is_err());
        assert_eq!(fs::read(&outside)?, ORIGINAL);
        Ok(())
    }
}
