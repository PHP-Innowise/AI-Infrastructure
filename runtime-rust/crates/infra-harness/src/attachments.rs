use crate::{
    HarnessError, Result,
    store::{self, SessionStore},
};
use base64::Engine;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    fs::OpenOptions,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};
pub const MAX_JSON_BYTES: usize = 12 * 1024 * 1024;
const MAX_ATTACHMENT_BYTES: usize = 4 * 1024 * 1024;
const ATTACHMENT_RACY_NS: i128 = 2_000_000_000;
const MAX_READ_CACHE_ENTRIES: usize = 128;
const MAX_READ_CACHE_BYTES: usize = 32 * 1024 * 1024;

type AttachmentCacheKey = (PathBuf, String);

#[derive(Clone)]
struct CachedAttachment {
    size: u64,
    mtime_ns: i128,
    dev: u64,
    ino: u64,
    bytes: Arc<Vec<u8>>,
}

#[derive(Default)]
struct AttachmentReadCache {
    entries: BTreeMap<AttachmentCacheKey, CachedAttachment>,
    order: VecDeque<AttachmentCacheKey>,
    total_bytes: usize,
}

impl AttachmentReadCache {
    fn touch(&mut self, key: &AttachmentCacheKey) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            self.order.remove(pos);
        }
        self.order.push_back(key.clone());
    }

    fn insert(&mut self, key: AttachmentCacheKey, entry: CachedAttachment) {
        if let Some(old) = self.entries.remove(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(old.bytes.len());
            if let Some(pos) = self.order.iter().position(|k| k == &key) {
                self.order.remove(pos);
            }
        }
        if entry.bytes.len() > MAX_READ_CACHE_BYTES {
            return;
        }
        while (self.entries.len() >= MAX_READ_CACHE_ENTRIES
            || self.total_bytes.saturating_add(entry.bytes.len()) > MAX_READ_CACHE_BYTES)
            && let Some(oldest) = self.order.pop_front()
        {
            if let Some(removed) = self.entries.remove(&oldest) {
                self.total_bytes = self.total_bytes.saturating_sub(removed.bytes.len());
            }
        }
        self.total_bytes = self.total_bytes.saturating_add(entry.bytes.len());
        self.order.push_back(key.clone());
        self.entries.insert(key, entry);
    }
}

fn attachment_read_cache() -> &'static Mutex<AttachmentReadCache> {
    static CACHE: OnceLock<Mutex<AttachmentReadCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(AttachmentReadCache::default()))
}

#[cfg(test)]
fn attachment_disk_read_counts() -> &'static Mutex<BTreeMap<PathBuf, usize>> {
    static COUNTS: OnceLock<Mutex<BTreeMap<PathBuf, usize>>> = OnceLock::new();
    COUNTS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn time_to_unix_ns(time: SystemTime) -> Option<i128> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(delta) => i128::try_from(delta.as_nanos()).ok(),
        Err(err) => i128::try_from(err.duration().as_nanos())
            .ok()
            .and_then(|ns| ns.checked_neg()),
    }
}

fn now_unix_ns() -> i128 {
    time_to_unix_ns(SystemTime::now()).unwrap_or(0)
}

fn file_stat_identity(meta: &std::fs::Metadata) -> (i128, u64, u64) {
    let mtime_ns = meta.modified().ok().and_then(time_to_unix_ns).unwrap_or(0);
    #[cfg(unix)]
    let (dev, ino) = {
        use std::os::unix::fs::MetadataExt;
        (meta.dev(), meta.ino())
    };
    #[cfg(not(unix))]
    let (dev, ino) = (0u64, 0u64);
    (mtime_ns, dev, ino)
}

fn racy_settled(mtime_ns: i128, now_ns: i128) -> bool {
    mtime_ns > 0 && now_ns > ATTACHMENT_RACY_NS && mtime_ns <= now_ns - ATTACHMENT_RACY_NS
}

fn read_bounded_regular(path: &Path, limit: usize) -> Result<Vec<u8>> {
    #[cfg(test)]
    if let Ok(mut counts) = attachment_disk_read_counts().lock() {
        *counts.entry(path.to_path_buf()).or_insert(0) += 1;
    }
    let before = std::fs::symlink_metadata(path)?;
    if infra_core::fs::linked(&before) || !before.is_file() {
        return Err(HarnessError::conflict(
            "The stored attachment changed. Attach the file again.",
        ));
    }
    if before.len() > limit as u64 {
        return Err(HarnessError::conflict(
            "The stored attachment changed. Attach the file again.",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = options.open(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() || file_stat_identity(&before) != file_stat_identity(&opened) {
        return Err(HarnessError::conflict(
            "The stored attachment changed. Attach the file again.",
        ));
    }
    let mut bytes = Vec::with_capacity(opened.len().min(limit as u64) as usize);
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    let after = std::fs::symlink_metadata(path)?;
    if bytes.len() > limit || file_stat_identity(&after) != file_stat_identity(&before) {
        return Err(HarnessError::conflict(
            "The stored attachment changed. Attach the file again.",
        ));
    }
    Ok(bytes)
}
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
    read_at(store, sid, id, now_unix_ns())
}

fn read_at(
    store: &SessionStore,
    sid: &str,
    id: &str,
    now_ns: i128,
) -> Result<(Value, Vec<u8>, PathBuf)> {
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
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(HarnessError::missing(
                "The stored attachment is unavailable.",
            ));
        }
        Err(error) => return Err(error.into()),
    };
    if infra_core::fs::linked(&meta) || !meta.is_file() || meta.len() != row.1 {
        return Err(HarnessError::conflict(
            "The stored attachment changed. Attach the file again.",
        ));
    }
    let (mtime_ns, dev, ino) = file_stat_identity(&meta);
    let settled = racy_settled(mtime_ns, now_ns);
    let cache_key = (path.clone(), row.2.clone());
    if settled && let Ok(mut guard) = attachment_read_cache().lock() {
        let hit = guard
            .entries
            .get(&cache_key)
            .filter(|entry| {
                entry.size == row.1
                    && entry.mtime_ns == mtime_ns
                    && entry.dev == dev
                    && entry.ino == ino
            })
            .map(|entry| entry.bytes.as_ref().clone());
        if let Some(bytes) = hit {
            guard.touch(&cache_key);
            return Ok((
                json!({"id":id,"name":row.0,"size":row.1,"digest":row.2}),
                bytes,
                path,
            ));
        }
    }
    let bytes = read_bounded_regular(&path, MAX_ATTACHMENT_BYTES)?;
    if bytes.len() as u64 != row.1 || store::sha(&bytes) != row.2 {
        return Err(HarnessError::conflict(
            "The stored attachment changed. Attach the file again.",
        ));
    }
    if settled && let Ok(mut guard) = attachment_read_cache().lock() {
        guard.insert(
            cache_key,
            CachedAttachment {
                size: row.1,
                mtime_ns,
                dev,
                ino,
                bytes: Arc::new(bytes.clone()),
            },
        );
    }
    Ok((
        json!({"id":id,"name":row.0,"size":row.1,"digest":row.2}),
        bytes,
        path,
    ))
}

pub fn directories(store: &SessionStore, sid: &str) -> Result<Vec<PathBuf>> {
    let root = store.state.join("attachments");
    let ids = {
        let db = store.connection()?;
        let mut query =
            db.prepare("SELECT id FROM attachments WHERE session_id=?1 ORDER BY rowid")?;
        query
            .query_map([sid], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    let mut folders = Vec::new();
    for id in ids {
        if id.len() != 32
            || !id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            continue;
        }
        let folder = root.join(&id);
        infra_core::fs::admit_descendant(&root, &folder)?;
        // An upload whose folder is gone is reported when its message reads it.
        if std::fs::symlink_metadata(&folder).is_ok_and(|metadata| metadata.is_dir()) {
            folders.push(folder);
        }
    }
    Ok(folders)
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
fn write_private_upload(file: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o400)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut output = options.open(file)?;
    output.write_all(bytes)?;
    output.sync_all()?;
    drop(output);
    infra_host::util::private_file(file)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o400))?;
    }
    Ok(())
}

fn ensure_cas_blob(root: &Path, cas_root: &Path, digest: &str, bytes: &[u8]) -> Result<PathBuf> {
    let blob = cas_root.join(digest);
    infra_core::fs::admit_descendant(root, &blob)?;
    if let Ok(meta) = std::fs::symlink_metadata(&blob) {
        let valid_existing = !infra_core::fs::linked(&meta)
            && meta.is_file()
            && meta.len() == bytes.len() as u64
            && read_bounded_regular(&blob, MAX_ATTACHMENT_BYTES)
                .is_ok_and(|existing| store::sha(&existing) == digest);
        if valid_existing {
            return Ok(blob);
        }
        let _ = std::fs::remove_file(&blob);
    }
    let temp = cas_root.join(format!(".tmp-{digest}-{}", store::hexid()));
    infra_core::fs::admit_descendant(cas_root, &temp)?;
    let write_res = write_private_upload(&temp, bytes);
    if let Err(err) = write_res {
        let _ = std::fs::remove_file(&temp);
        return Err(err);
    }
    match std::fs::hard_link(&temp, &blob) {
        Ok(()) => {
            let _ = std::fs::remove_file(&temp);
        }
        Err(_) => {
            if let Err(err) = std::fs::rename(&temp, &blob) {
                let _ = std::fs::remove_file(&temp);
                if let Ok(meta) = std::fs::symlink_metadata(&blob)
                    && !infra_core::fs::linked(&meta)
                    && meta.is_file()
                    && meta.len() == bytes.len() as u64
                {
                    return Ok(blob);
                }
                return Err(err.into());
            }
        }
    }
    Ok(blob)
}

pub fn stage(
    db: &rusqlite::Connection,
    state: &Path,
    sid: &str,
    uploads: &[Upload],
) -> Result<(Vec<Value>, UploadGuard)> {
    let root = state.join("attachments");
    infra_host::util::private_directory(&root)?;
    let cas_root = root.join("_cas");
    infra_host::util::private_directory(&cas_root)?;
    let mut guard = UploadGuard {
        folders: Vec::new(),
        committed: false,
    };
    let mut saved = Vec::new();
    for upload in uploads {
        let digest = store::sha(&upload.bytes);
        if let Some(existing_id) = db
            .query_row(
                "SELECT id FROM attachments WHERE session_id=?1 AND name=?2 AND size=?3 AND digest=?4 ORDER BY rowid LIMIT 1",
                params![sid, upload.name, upload.bytes.len() as i64, digest],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        {
            let existing_path = root
                .join(&existing_id)
                .join(format!("upload-{}", upload.name));
            if infra_core::fs::admit_descendant(&root, &existing_path).is_ok()
                && std::fs::symlink_metadata(&existing_path).is_ok_and(|meta| {
                    !infra_core::fs::linked(&meta)
                        && meta.is_file()
                        && meta.len() == upload.bytes.len() as u64
                })
                && read_bounded_regular(&existing_path, MAX_ATTACHMENT_BYTES)
                    .is_ok_and(|bytes| store::sha(&bytes) == digest)
            {
                saved.push(json!({"id":existing_id,"name":upload.name,"size":upload.bytes.len()}));
                continue;
            }
        }
        let id = store::hexid();
        let folder = root.join(&id);
        infra_core::fs::admit_descendant(&root, &folder)?;
        std::fs::create_dir(&folder)?;
        infra_host::util::private_directory(&folder)?;
        guard.folders.push(folder.clone());
        let file = folder.join(format!("upload-{}", upload.name));
        let linked = ensure_cas_blob(&root, &cas_root, &digest, &upload.bytes)
            .ok()
            .and_then(|blob| std::fs::hard_link(blob, &file).ok())
            .is_some();
        if !linked {
            write_private_upload(&file, &upload.bytes)?;
        }
        db.execute(
            "INSERT INTO attachments VALUES(?1,?2,?3,?4,?5)",
            params![id, sid, upload.name, upload.bytes.len() as i64, digest],
        )?;
        saved.push(json!({"id":id,"name":upload.name,"size":upload.bytes.len()}));
    }
    Ok((saved, guard))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachments_stage_deduplicates_and_read_uses_racy_ns_stat_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let state = tmp.path().join("state");
        let store = SessionStore::open(&state).unwrap();
        let proj = tmp.path().join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let reg = store.register(&proj).unwrap();
        let pid = reg["id"].as_str().unwrap();
        let s1 = store
            .create(
                &json!({"project_id": pid, "provider": "claude", "mode": "edit"}),
                "Session 1",
                &[],
            )
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let s2 = store
            .create(
                &json!({"project_id": pid, "provider": "claude", "mode": "edit"}),
                "Session 2",
                &[],
            )
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();

        let payload = b"shared-attachment-payload-bytes".to_vec();
        let saved_1 = save(
            &store,
            &s1,
            vec![Upload {
                name: "spec.md".into(),
                bytes: payload.clone(),
            }],
        )
        .unwrap();
        let id1 = saved_1[0]["id"].as_str().unwrap().to_owned();

        // Same session + identical (name, bytes) reuses existing hexid folder without allocating a new one.
        let saved_1_dup = save(
            &store,
            &s1,
            vec![Upload {
                name: "spec.md".into(),
                bytes: payload.clone(),
            }],
        )
        .unwrap();
        assert_eq!(saved_1_dup[0]["id"], id1);

        // Different session + identical payload reuses content-addressed CAS storage (hardlinked on Unix).
        let saved_2 = save(
            &store,
            &s2,
            vec![Upload {
                name: "copy.md".into(),
                bytes: payload.clone(),
            }],
        )
        .unwrap();
        let id2 = saved_2[0]["id"].as_str().unwrap().to_owned();
        assert_ne!(id1, id2);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let p1 = store
                .state
                .join("attachments")
                .join(&id1)
                .join("upload-spec.md");
            let p2 = store
                .state
                .join("attachments")
                .join(&id2)
                .join("upload-copy.md");
            assert_eq!(
                std::fs::symlink_metadata(&p1).unwrap().ino(),
                std::fs::symlink_metadata(&p2).unwrap().ino(),
                "identical payloads across sessions must share CAS inode via hardlink"
            );
        }

        let path1 = store
            .state
            .join("attachments")
            .join(&id1)
            .join("upload-spec.md");
        let (mtime_ns, _, _) = file_stat_identity(&std::fs::symlink_metadata(&path1).unwrap());

        let reads_for = |path: &Path| -> usize {
            attachment_disk_read_counts()
                .lock()
                .unwrap()
                .get(path)
                .copied()
                .unwrap_or(0)
        };

        // Inside 2s RACY_NS window: never populates or hits read cache.
        let r0 = reads_for(&path1);
        let (_, bytes_racy, _) =
            read_at(&store, &s1, &id1, mtime_ns + ATTACHMENT_RACY_NS - 1).unwrap();
        assert_eq!(bytes_racy, payload);
        let r1 = reads_for(&path1);
        assert_eq!(r1, r0 + 1);

        // Settled (> RACY_NS): first call reads & caches; second call hits stat cache with 0 disk reads!
        let settled_now = mtime_ns + ATTACHMENT_RACY_NS + 100;
        let (_, bytes_s1, _) = read_at(&store, &s1, &id1, settled_now).unwrap();
        assert_eq!(bytes_s1, payload);
        let r2 = reads_for(&path1);
        assert_eq!(r2, r1 + 1);
        let (_, bytes_s2, _) = read_at(&store, &s1, &id1, settled_now + 50).unwrap();
        assert_eq!(bytes_s2, payload);
        let r3 = reads_for(&path1);
        assert_eq!(r3, r2, "settled attachment read must hit stat cache");

        // Second session attachment also reads cleanly even with nlink > 1.
        let (meta2, bytes_2, _) = read_at(&store, &s2, &id2, settled_now).unwrap();
        assert_eq!(meta2["name"], "copy.md");
        assert_eq!(bytes_2, payload);
    }
}
