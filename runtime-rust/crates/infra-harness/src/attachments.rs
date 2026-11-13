use crate::{
    HarnessError, Result,
    store::{self, SessionStore},
};
use base64::Engine;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};
pub const MAX_JSON_BYTES: usize = 12 * 1024 * 1024;
pub struct Upload {
    pub name: String,
    pub bytes: Vec<u8>,
}
pub fn validate(value: &Value) -> Result<Vec<Upload>> {
    let items = value
        .as_array()
        .ok_or_else(|| HarnessError::invalid("Attachments must be a list."))?;
    if items.len() > 5 {
        return Err(HarnessError::invalid("Attach up to 5 files per message."));
    }
    let mut size = 0;
    let mut out = Vec::new();
    for item in items {
        if !store::exact(item, &["name", "data"], &[]) {
            return Err(HarnessError::invalid(
                "Each attachment requires name and base64 data.",
            ));
        }
        let name = item["name"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Invalid attachment filename."))?;
        if name.is_empty()
            || name.len() > 200
            || name.chars().any(|c| c.is_control() || "/\\:".contains(c))
            || [".", ".."].contains(&name)
            || name.ends_with('.')
            || name.ends_with(' ')
        {
            return Err(HarnessError::invalid(
                "Use an attachment filename without paths or control characters.",
            ));
        }
        let data = item["data"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Invalid attachment base64."))?;
        if data.len() > 4 * (4_usize * 1024 * 1024).div_ceil(3) {
            return Err(HarnessError::invalid(
                "Each attachment must be at most 4 MiB.",
            ));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| HarnessError::invalid("Invalid attachment base64."))?;
        size += bytes.len();
        if bytes.len() > 4 * 1024 * 1024 || size > 8 * 1024 * 1024 {
            return Err(HarnessError::invalid(
                "Attachments must be at most 4 MiB each and 8 MiB per message.",
            ));
        }
        out.push(Upload {
            name: name.into(),
            bytes,
        });
    }
    Ok(out)
}
pub fn save(store: &SessionStore, sid: &str, uploads: Vec<Upload>) -> Result<Vec<Value>> {
    let mut db = store.connection()?;
    let tx = db.transaction()?;
    let (saved, mut guard) = stage(&tx, &store.state, sid, &uploads)?;
    tx.commit()?;
    guard.commit();
    Ok(saved)
}
pub fn read(store: &SessionStore, sid: &str, id: &str) -> Result<(Value, Vec<u8>, PathBuf)> {
    if id.len() != 32
        || !id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(HarnessError::invalid("Invalid attachment ID."));
    }
    let row = store
        .connection()?
        .query_row(
            "SELECT name,size,digest FROM attachments WHERE id=?1 AND session_id=?2",
            params![id, sid],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| HarnessError::missing("Attachment not found in this session."))?;
    let path = store
        .state
        .join("attachments")
        .join(id)
        .join(format!("upload-{}", row.0));
    infra_core::fs::admit_descendant(&store.state, &path)?;
    let root = store.state.join("attachments");
    let mut files = infra_core::bank::publication::PublicationFiles::new(&root)?;
    let relative = format!("{id}/upload-{}", row.0);
    let bytes = files
        .read(&relative, 4 * 1024 * 1024)?
        .ok_or_else(|| HarnessError::missing("The stored attachment is unavailable."))?;
    if bytes.len() as u64 != row.1 || store::sha(&bytes) != row.2 {
        return Err(HarnessError::conflict(
            "The stored attachment changed. Attach the file again.",
        ));
    }
    Ok((
        json!({"id":id,"name":row.0,"size":row.1,"digest":row.2}),
        bytes,
        path,
    ))
}

pub struct UploadGuard {
    pub folders: Vec<PathBuf>,
    pub committed: bool,
}
impl UploadGuard {
    pub fn commit(&mut self) {
        self.committed = true;
    }
}
impl Drop for UploadGuard {
    fn drop(&mut self) {
        if !self.committed {
            for folder in &self.folders {
                #[cfg(windows)]
                #[expect(
                    clippy::permissions_set_readonly_false,
                    reason = "Owned upload cleanup clears the Windows ReadOnly attribute; this block is excluded on Unix."
                )]
                {
                    if let Ok(entries) = std::fs::read_dir(folder) {
                        for entry in entries.flatten() {
                            if entry
                                .file_name()
                                .to_str()
                                .is_some_and(|n| n.starts_with("upload-"))
                                && let Ok(metadata) = std::fs::symlink_metadata(entry.path())
                                && metadata.is_file()
                                && !infra_core::fs::linked(&metadata)
                            {
                                let mut mode = metadata.permissions();
                                mode.set_readonly(false);
                                let _ = std::fs::set_permissions(entry.path(), mode);
                            }
                        }
                    }
                }
                let _ = std::fs::remove_dir_all(folder);
            }
        }
    }
}
pub fn stage(
    db: &rusqlite::Connection,
    state: &Path,
    sid: &str,
    uploads: &[Upload],
) -> Result<(Vec<Value>, UploadGuard)> {
    let root = state.join("attachments");
    infra_host::util::private_directory(&root)?;
    let mut guard = UploadGuard {
        folders: Vec::new(),
        committed: false,
    };
    let mut saved = Vec::new();
    for upload in uploads {
        let id = store::hexid();
        let folder = root.join(&id);
        infra_core::fs::admit_descendant(&root, &folder)?;
        std::fs::create_dir(&folder)?;
        infra_host::util::private_directory(&folder)?;
        guard.folders.push(folder.clone());
        let file = folder.join(format!("upload-{}", upload.name));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o400)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        let mut output = options.open(&file)?;
        output.write_all(&upload.bytes)?;
        output.sync_all()?;
        drop(output);
        infra_host::util::private_file(&file)?;
        let mut rooted = infra_core::bank::publication::PublicationFiles::new(&folder)?;
        rooted.set_mode(&format!("upload-{}", upload.name), 0o400)?;
        db.execute(
            "INSERT INTO attachments VALUES(?1,?2,?3,?4,?5)",
            params![
                id,
                sid,
                upload.name,
                upload.bytes.len() as i64,
                store::sha(&upload.bytes)
            ],
        )?;
        saved.push(json!({"id":id,"name":upload.name,"size":upload.bytes.len()}));
    }
    Ok((saved, guard))
}
