//! Preserve Git's recorded blob when changing executable bits.
use crate::common;
use infra_core::{Result, RuntimeError, git::GitFrame};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug)]
struct Entry {
    mode: String,
    blob: String,
}
pub struct FileModes {
    root: PathBuf,
    index: BTreeMap<String, Entry>,
    trust_disk: bool,
    pending: BTreeMap<String, String>,
}
fn source_git(mut error: RuntimeError) -> RuntimeError {
    error.code = Some("source-git".into());
    error
}

/// Empty ordinary marker directories grant no Git authority. Every other
/// nearest marker must resolve to its own valid canonical Git frame.
fn admitted_frame(root: &Path) -> Result<Option<GitFrame>> {
    let canonical = infra_core::fs::canonicalize(root).map_err(|error| source_git(error.into()))?;
    if !canonical.is_dir() {
        return Err(source_git(RuntimeError::unavailable(
            "File-mode source is not a directory",
        )));
    }
    let mut checkout = None;
    for directory in canonical.ancestors() {
        let marker = directory.join(".git");
        let metadata = match fs::symlink_metadata(&marker) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(source_git(error.into())),
        };
        if infra_core::fs::linked(&metadata)
            || infra_core::fs::linked_at(&marker, &metadata).map_err(source_git)?
        {
            return Err(source_git(RuntimeError::policy(
                "Redirecting Git metadata cannot grant file-mode authority",
            )));
        }
        if metadata.is_dir() {
            match fs::read_dir(&marker)
                .map_err(|error| source_git(error.into()))?
                .next()
            {
                None => continue,
                Some(Err(error)) => return Err(source_git(error.into())),
                Some(Ok(_)) => {}
            }
        } else if !metadata.is_file() {
            return Err(source_git(RuntimeError::policy(
                "Git metadata is not an ordinary directory or gitdir file",
            )));
        }
        checkout = Some(directory.to_owned());
        break;
    }
    let Some(checkout) = checkout else {
        return Ok(None);
    };
    let frame = GitFrame::discover(&canonical)?.ok_or_else(|| {
        source_git(RuntimeError::unavailable(
            "Existing Git metadata has no valid source frame",
        ))
    })?;
    if frame.top != checkout {
        return Err(source_git(RuntimeError::unavailable(
            "Nearest Git metadata differs from its canonical source frame",
        )));
    }
    Ok(Some(frame))
}

impl FileModes {
    pub fn read(root: &Path) -> Result<Self> {
        let Some(frame) = admitted_frame(root)? else {
            return Ok(Self {
                root: root.into(),
                index: BTreeMap::new(),
                trust_disk: cfg!(unix),
                pending: BTreeMap::new(),
            });
        };
        let raw = common::git_ok(
            root,
            &["ls-files", "--stage", "-z", "--full-name", "--", "."],
            None,
        )
        .map_err(source_git)?;
        let prefix = frame.prefix.as_str();
        let mut index = BTreeMap::new();
        for record in raw
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
        {
            let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
                return Err(RuntimeError::invalid("Malformed Git index entry"));
            };
            let fields = std::str::from_utf8(&record[..tab])
                .map_err(|_| RuntimeError::invalid("Invalid Git index metadata"))?
                .split_whitespace()
                .collect::<Vec<_>>();
            if fields.len() != 3 {
                return Err(RuntimeError::invalid("Malformed Git index metadata"));
            }
            if fields[2] != "0" {
                continue;
            }
            let path = std::str::from_utf8(&record[tab + 1..])
                .map_err(|_| RuntimeError::invalid("Index path is not UTF-8"))?;
            if let Some(name) = path.strip_prefix(prefix) {
                common::relative(name)?;
                index.insert(
                    name.into(),
                    Entry {
                        mode: fields[0].into(),
                        blob: fields[1].into(),
                    },
                );
            }
        }
        let added = common::git_ok(
            root,
            &[
                "diff-files",
                "-z",
                "--name-only",
                "--diff-filter=A",
                "--",
                ".",
            ],
            None,
        )
        .map_err(source_git)?;
        for raw in added.split(|byte| *byte == 0).filter(|raw| !raw.is_empty()) {
            let path = std::str::from_utf8(raw)
                .map_err(|_| RuntimeError::invalid("Index path is not UTF-8"))?;
            if let Some(path) = path.strip_prefix(prefix) {
                index.remove(path);
            }
        }
        let configured =
            common::git(root, &["config", "--bool", "core.fileMode"], None).map_err(source_git)?;
        let trust_disk = match (configured.code, configured.stdout.as_slice()) {
            (0, b"true\n") | (1, b"") => cfg!(unix),
            (0, b"false\n") => false,
            _ => {
                return Err(source_git(RuntimeError::unavailable(
                    "Git core.fileMode query failed or returned an invalid boolean",
                )));
            }
        };
        Ok(Self {
            root: root.into(),
            index,
            trust_disk,
            pending: BTreeMap::new(),
        })
    }
    pub fn executable(&self, name: &str) -> Result<Option<bool>> {
        common::relative(name)?;
        if let Some(entry) = self.index.get(name) {
            return Ok(Some(entry.mode == "100755"));
        }
        if !self.trust_disk {
            return Ok(None);
        }
        let image = fs::symlink_metadata(common::under(&self.root, name)?)?;
        if infra_core::fs::linked(&image) || !image.is_file() {
            return Err(RuntimeError::policy("Executable source is not regular"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            Ok(Some(image.permissions().mode() & 0o111 != 0))
        }
        #[cfg(not(unix))]
        {
            Ok(None)
        }
    }
    pub fn set(&mut self, name: &str, executable: bool) -> Result<bool> {
        let path = common::under(&self.root, name)?;
        #[cfg(not(unix))]
        let _ = &path;
        let mut changed = false;
        #[cfg(unix)]
        if self.trust_disk {
            use std::os::unix::fs::PermissionsExt;
            let image = fs::symlink_metadata(&path)?;
            if infra_core::fs::linked(&image) || !image.is_file() {
                return Err(RuntimeError::policy(
                    "Executable destination is not regular",
                ));
            }
            let current = image.permissions().mode();
            let wanted = if executable {
                current | ((current & 0o444) >> 2)
            } else {
                current & !0o111
            };
            if current != wanted {
                fs::set_permissions(path, fs::Permissions::from_mode(wanted))?;
                changed = true;
            }
        }
        let mode = if executable { "100755" } else { "100644" };
        if self.index.get(name).is_some_and(|entry| entry.mode != mode) {
            self.pending.insert(name.into(), mode.into());
            changed = true;
        }
        Ok(changed)
    }
    pub fn flush(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let prefix = common::text(common::git_ok(
            &self.root,
            &["rev-parse", "--show-prefix"],
            None,
        )?)?;
        let prefix = prefix.strip_suffix('\n').unwrap_or(&prefix);
        let pending = std::mem::take(&mut self.pending)
            .into_iter()
            .collect::<Vec<_>>();
        for batch in pending.chunks(200) {
            let mut input = Vec::new();
            for (name, mode) in batch {
                let entry = self
                    .index
                    .get(name)
                    .ok_or_else(|| RuntimeError::conflict("Index entry disappeared"))?;
                input.extend_from_slice(
                    format!("{mode} {}\t{prefix}{name}\0", entry.blob).as_bytes(),
                );
            }
            common::git_ok(
                &self.root,
                &["update-index", "-z", "--index-info"],
                Some(input),
            )?;
            for (name, mode) in batch {
                if let Some(entry) = self.index.get_mut(name) {
                    entry.mode = mode.clone();
                }
            }
        }
        Ok(())
    }
}
