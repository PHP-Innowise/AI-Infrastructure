use super::{
    BankService, codec,
    publication::{self, PLAN_LIMIT, PublicationFiles},
    validation,
};
use crate::{
    Result, RuntimeError,
    domain::{self, json},
};
use serde_json::{Value, json as object};
use std::path::Path;

pub const CHECKPOINT: &str = "project-brain/control/bank-retirement.json";
const PROFILE: &str = "bank-retirement-v1";
type Part = (String, Vec<u8>, Vec<u8>);
pub(crate) fn strict(value: &Value, keys: &[&str]) -> Result<()> {
    if value.as_object().is_none_or(|object| {
        object.len() != keys.len() || keys.iter().any(|key| !object.contains_key(*key))
    }) {
        return Err(RuntimeError::invalid(
            "Custody object has unexpected or missing fields",
        ));
    }
    Ok(())
}

fn parts(plan: &Value) -> Result<Vec<Part>> {
    strict(
        plan,
        &[
            "profile",
            "memory_id",
            "valid_to",
            "superseded_by",
            "chunks",
            "sha256",
        ],
    )?;
    publication::verify_seal(plan)?;
    if plan["profile"] != PROFILE {
        return Err(RuntimeError::invalid(
            "Unknown retirement checkpoint profile",
        ));
    }
    let id = plan["memory_id"]
        .as_str()
        .ok_or_else(|| RuntimeError::invalid("Invalid retirement identity"))?;
    let successor = plan["superseded_by"].as_str();
    if !plan["superseded_by"].is_null() && successor.is_none() || successor == Some(id) {
        return Err(RuntimeError::invalid("Invalid retirement successor"));
    }
    if codec::date(plan["valid_to"].as_str().unwrap_or(""))?.to_string()
        != plan["valid_to"].as_str().unwrap_or("")
    {
        return Err(RuntimeError::invalid(
            "Retirement checkpoint date is not canonical",
        ));
    }
    let rows = plan["chunks"]
        .as_array()
        .ok_or_else(|| RuntimeError::invalid("Retirement chunks must be a list"))?;
    if rows.len() != if successor.is_some() { 2 } else { 1 } {
        return Err(RuntimeError::invalid("Retirement image count differs"));
    }
    let mut output = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        strict(row, &["path", "before", "after"])?;
        let path = row["path"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("Invalid retirement path"))?;
        let wanted_id = if index == 0 {
            id
        } else {
            successor.unwrap_or("")
        };
        let filename = Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if Path::new(path).parent() != Some(Path::new("memory-bank/chunks"))
            || path.contains('\\')
            || codec::filename_id(filename)?.as_deref() != Some(wanted_id)
        {
            return Err(RuntimeError::policy(
                "Retirement path differs from identity",
            ));
        }
        let before = publication::unblob(&row["before"])?;
        let after = publication::unblob(&row["after"])?;
        let (mut metadata, body) = codec::parse_raw(&before)?;
        if metadata["id"].as_str() != Some(wanted_id) {
            return Err(RuntimeError::invalid("Retirement image identity differs"));
        }
        if index == 0 {
            metadata["status"] = if successor.is_some() {
                "superseded"
            } else {
                "archived"
            }
            .into();
            metadata["superseded_by"] = plan["superseded_by"].clone();
            metadata["valid_to"] = plan["valid_to"].clone();
        } else {
            let mut previous = codec::strings(&metadata, "supersedes", true)?;
            if !previous.iter().any(|previous| previous == id) {
                previous.push(id.into());
            }
            metadata["supersedes"] = object!(previous);
        }
        if codec::render_chunk(&metadata, &body)? != after {
            return Err(RuntimeError::policy(
                "Retirement changes unrelated knowledge",
            ));
        }
        output.push((path.into(), before, after));
    }
    Ok(output)
}

enum IndexBefore {
    Regenerate,
    Restore(Option<Vec<u8>>),
}
fn rollback(
    service: &BankService,
    raw: &[u8],
    parts: &[Part],
    index_before: IndexBefore,
) -> Result<()> {
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    rollback_in(&mut files, raw, parts, index_before)
}
fn rollback_in(
    files: &mut PublicationFiles,
    raw: &[u8],
    parts: &[Part],
    index_before: IndexBefore,
) -> Result<()> {
    if files.read(CHECKPOINT, 2 * PLAN_LIMIT)?.as_deref() != Some(raw) {
        return Err(RuntimeError::conflict(
            "Retirement checkpoint changed before recovery",
        ));
    }
    let mut states = Vec::new();
    for (path, before, after) in parts {
        let current = files.read(path, publication::IMAGE_LIMIT)?;
        if current.as_deref() != Some(before.as_slice())
            && current.as_deref() != Some(after.as_slice())
        {
            return Err(RuntimeError::conflict(
                "Retirement image was externally edited; preserve and review recovery",
            ));
        }
        states.push((path, before, current));
    }
    for (path, before, current) in states {
        if current.as_deref() != Some(before.as_slice()) {
            files.write_expected(path, before, current.as_deref())?;
        }
    }
    for (path, before, _) in parts {
        if files.read(path, publication::IMAGE_LIMIT)?.as_deref() != Some(before.as_slice()) {
            return Err(RuntimeError::conflict(
                "Retirement image changed during recovery",
            ));
        }
    }
    let current = files.read("memory-bank/INDEX.md", PLAN_LIMIT)?;
    let replacement = match index_before {
        IndexBefore::Regenerate => Some(codec::render_index_in(files)?.0.into_bytes()),
        IndexBefore::Restore(previous) => previous,
    };
    if current != replacement {
        match replacement {
            Some(bytes) => {
                files.write_expected("memory-bank/INDEX.md", &bytes, current.as_deref())?
            }
            None => {
                if let Some(current) = current {
                    files.unlink_expected("memory-bank/INDEX.md", &current)?;
                }
            }
        }
    }
    for (path, before, _) in parts {
        if files.read(path, publication::IMAGE_LIMIT)?.as_deref() != Some(before.as_slice()) {
            return Err(RuntimeError::conflict(
                "Retirement image changed before clearing recovery",
            ));
        }
    }
    files.unlink_expected(CHECKPOINT, raw)
}

pub fn recover_pending(repository: &Path) -> Result<()> {
    match std::fs::symlink_metadata(repository.join(CHECKPOINT)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    let mut files = PublicationFiles::new(repository)?;
    recover_pending_in(&mut files)
}
/// Recovery before attached-state seeding must use the same retained state root.
pub fn recover_pending_in(files: &mut PublicationFiles) -> Result<()> {
    let Some(raw) = files.read(CHECKPOINT, 2 * PLAN_LIMIT)? else {
        return Ok(());
    };
    let plan = json::parse(
        std::str::from_utf8(&raw)
            .map_err(|_| RuntimeError::invalid("Retirement checkpoint encoding is invalid"))?,
    )?;
    rollback_in(files, &raw, &parts(&plan)?, IndexBefore::Regenerate)
}

pub fn retire(
    service: &BankService,
    id: &str,
    valid_to: &str,
    successor: Option<&str>,
    reason: Option<&str>,
) -> Result<Value> {
    let closing = codec::date(valid_to)?;
    if successor == Some(id) {
        return Err(RuntimeError::invalid("A chunk cannot supersede itself"));
    }
    let _guard = service.brain()?.lock()?;
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    let target = codec::find(service, id, &mut files)?;
    let mut metadata = target.metadata.clone();
    metadata["status"] = if successor.is_some() {
        "superseded"
    } else {
        "archived"
    }
    .into();
    metadata["superseded_by"] = object!(successor);
    metadata["valid_to"] = closing.to_string().into();
    let mut rows = vec![
        object!({"path":target.path,"before":publication::blob(&target.raw)?,"after":publication::blob(&codec::render_chunk(&metadata,&target.body)?)?}),
    ];
    let mut attested = vec![target.path.clone()];
    if let Some(successor) = successor {
        let next = codec::find(service, successor, &mut files)?;
        let mut m = next.metadata.clone();
        let mut supersedes = codec::strings(&m, "supersedes", true)?;
        if !supersedes.iter().any(|previous| previous == id) {
            supersedes.push(id.into());
        }
        m["supersedes"] = object!(supersedes);
        rows.push(object!({"path":next.path,"before":publication::blob(&next.raw)?,"after":publication::blob(&codec::render_chunk(&m,&next.body)?)?}));
        attested.push(next.path);
    }
    let before_errors = validation::validate_bank(service)?;
    let mut plan = object!({"profile":PROFILE,"memory_id":id,"valid_to":closing.to_string(),"superseded_by":successor,"chunks":rows});
    publication::seal(&mut plan)?;
    let images = parts(&plan)?;
    let raw = format!("{}\n", json::sorted_pretty(&plan)?).into_bytes();
    if raw.len() > 2 * PLAN_LIMIT {
        return Err(RuntimeError::invalid(
            "Retirement checkpoint exceeds byte limit",
        ));
    }
    let index_before = files.read("memory-bank/INDEX.md", PLAN_LIMIT)?;
    files.write_expected(CHECKPOINT, &raw, None)?;
    let mut chunks = 0;
    let result = (|| -> Result<()> {
        for (path, before, after) in &images {
            if before != after {
                files.write_expected(path, after, Some(before))?;
            }
        }
        let (rendered, count) = codec::render_index(service)?;
        chunks = count;
        files.write_expected(
            "memory-bank/INDEX.md",
            rendered.as_bytes(),
            index_before.as_deref(),
        )?;
        let errors = validation::write_errors(service, &before_errors, &attested)?;
        if !errors.is_empty() {
            return Err(RuntimeError::invalid(format!(
                "Memory retirement failed validation: {}",
                errors.join("; ")
            )));
        }
        for (path, _, after) in &images {
            if files.read(path, publication::IMAGE_LIMIT)?.as_deref() != Some(after.as_slice()) {
                return Err(RuntimeError::conflict(
                    "Retirement image changed before commit",
                ));
            }
        }
        files.unlink_expected(CHECKPOINT, &raw)
    })();
    if let Err(error) = result {
        if files.read(CHECKPOINT, 2 * PLAN_LIMIT)?.is_some() {
            rollback(service, &raw, &images, IndexBefore::Restore(index_before))?;
        }
        return Err(error);
    }
    Ok(
        object!({"id":id,"status":metadata["status"],"valid_to":closing.to_string(),"superseded_by":successor,"reason":reason,"chunk":target.path,"index":"memory-bank/INDEX.md","chunks":chunks}),
    )
}

pub fn reverify(service: &BankService, id: &str, review_after: Option<&str>) -> Result<Value> {
    let today = codec::today();
    let review = match review_after {
        Some(value) => codec::date(value)?,
        None => today + chrono::Duration::days(365),
    };
    let _guard = service.brain()?.lock()?;
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    let chunk = codec::find(service, id, &mut files)?;
    if chunk.metadata["status"] != "active" {
        return Err(RuntimeError::policy(
            "Only an active chunk can be re-verified",
        ));
    }
    let before_errors = validation::validate_bank(service)?;
    let before_index = files.read("memory-bank/INDEX.md", PLAN_LIMIT)?;
    let mut metadata = chunk.metadata.clone();
    metadata["last_verified"] = today.to_string().into();
    metadata["review_after"] = review.to_string().into();
    let mut digests = Vec::new();
    for source in domain::digestible_sources(&metadata["sources"]) {
        if let Ok(rows) = domain::source_fingerprints(&service.context.roots, &[source]) {
            digests.extend(rows);
        }
    }
    metadata["source_digests"] = object!(digests);
    let after = codec::render_chunk(&metadata, &chunk.body)?;
    let mut count = 0;
    let result = (|| -> Result<()> {
        files.write_expected(&chunk.path, &after, Some(&chunk.raw))?;
        let (rendered, chunks) = codec::render_index(service)?;
        count = chunks;
        files.write_expected(
            "memory-bank/INDEX.md",
            rendered.as_bytes(),
            before_index.as_deref(),
        )?;
        let errors =
            validation::write_errors(service, &before_errors, std::slice::from_ref(&chunk.path))?;
        if !errors.is_empty() {
            return Err(RuntimeError::invalid(format!(
                "Memory re-verification failed: {}",
                errors.join("; ")
            )));
        }
        Ok(())
    })();
    if let Err(error) = result {
        let current = files.read(&chunk.path, publication::IMAGE_LIMIT)?;
        if current.as_deref() == Some(after.as_slice()) {
            files.write_expected(&chunk.path, &chunk.raw, current.as_deref())?;
        }
        let current = files.read("memory-bank/INDEX.md", PLAN_LIMIT)?;
        match before_index {
            Some(before) => {
                if current.as_deref() != Some(before.as_slice()) {
                    files.write_expected("memory-bank/INDEX.md", &before, current.as_deref())?;
                }
            }
            None => {
                if let Some(current) = current {
                    files.unlink_expected("memory-bank/INDEX.md", &current)?;
                }
            }
        }
        return Err(error);
    }
    Ok(
        object!({"id":id,"last_verified":today.to_string(),"review_after":review.to_string(),"source_digests":metadata["source_digests"],"chunk":chunk.path,"chunks":count}),
    )
}

#[cfg(test)]
mod attached_recovery_tests {
    use super::*;
    use crate::{Roots, bank::publication::PublicationFiles};
    use std::{fs, path::PathBuf};

    struct Fixture {
        root: PathBuf,
        roots: Roots,
        chunk: String,
        before: Vec<u8>,
        plan: Value,
        raw: Vec<u8>,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("infra-attach-recovery-{}", domain::uuid4()));
            fs::create_dir(&root).unwrap();
            let root = fs::canonicalize(root).unwrap();
            let tooling = root.join("tooling");
            let project = root.join("project");
            let state = root.join("state");
            fs::create_dir_all(tooling.join("memory-bank/.install")).unwrap();
            fs::create_dir(&project).unwrap();
            fs::create_dir_all(state.join("memory-bank/chunks")).unwrap();
            fs::create_dir_all(state.join("project-brain/control")).unwrap();
            fs::write(
                tooling.join("memory-bank/.install/INDEX.md"),
                format!("SEED MUST NOT PRECEDE RECOVERY\n{}", codec::HEADER),
            )
            .unwrap();
            let metadata = object!({
                "id":"MEM-0001","title":"Preserved recovery knowledge","type":"rule",
                "scope":["project"],"tags":[],"status":"active",
                "last_verified":"2026-01-01","valid_to":null,"superseded_by":null
            });
            let before = codec::render_chunk(&metadata, "Verified body.\n").unwrap();
            let mut retired = metadata.clone();
            retired["status"] = "archived".into();
            retired["valid_to"] = "2026-01-02".into();
            let after = codec::render_chunk(&retired, "Verified body.\n").unwrap();
            let chunk = "memory-bank/chunks/MEM-0001-preserved.md".to_string();
            let mut plan = object!({
                "profile":PROFILE,"memory_id":"MEM-0001","valid_to":"2026-01-02",
                "superseded_by":null,
                "chunks":[{"path":chunk,"before":publication::blob(&before).unwrap(),"after":publication::blob(&after).unwrap()}]
            });
            publication::seal(&mut plan).unwrap();
            let raw = format!("{}\n", json::sorted_pretty(&plan).unwrap()).into_bytes();
            fs::write(state.join(&chunk), &after).unwrap();
            fs::write(state.join(CHECKPOINT), &raw).unwrap();
            Self {
                root,
                roots: Roots {
                    tooling,
                    project,
                    state,
                    shared: false,
                },
                chunk,
                before,
                plan,
                raw,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn attached_initialization_recovers_before_index_seed_even_under_a_reentrant_lock() {
        let fixture = Fixture::new();
        assert!(!fixture.roots.state.join("memory-bank/INDEX.md").exists());
        let mut files = PublicationFiles::open_private_root(&fixture.roots.state).unwrap();
        let outer = files.acquire_private_state_lock().unwrap();
        fixture.roots.ensure_attached_state_in(&mut files).unwrap();
        assert_eq!(
            fs::read(fixture.roots.state.join(&fixture.chunk)).unwrap(),
            fixture.before
        );
        assert!(!fixture.roots.state.join(CHECKPOINT).exists());
        let index = fs::read_to_string(fixture.roots.state.join("memory-bank/INDEX.md")).unwrap();
        assert!(index.starts_with("# Memory Index\n\nRead [README.md]"));
        assert!(index.contains("MEM-0001") && index.contains("| active |"));
        assert!(!index.contains("SEED MUST NOT PRECEDE RECOVERY"));
        drop(outer);
    }

    #[test]
    fn held_custody_rollback_preserves_an_originally_absent_index() {
        let fixture = Fixture::new();
        fs::write(
            fixture.roots.state.join("memory-bank/INDEX.md"),
            b"partial derived index",
        )
        .unwrap();
        let mut files = PublicationFiles::open_private_root(&fixture.roots.state).unwrap();
        let _guard = files.acquire_private_state_lock().unwrap();
        rollback_in(
            &mut files,
            &fixture.raw,
            &parts(&fixture.plan).unwrap(),
            IndexBefore::Restore(None),
        )
        .unwrap();
        assert_eq!(
            fs::read(fixture.roots.state.join(&fixture.chunk)).unwrap(),
            fixture.before
        );
        assert!(!fixture.roots.state.join("memory-bank/INDEX.md").exists());
        assert!(!fixture.roots.state.join(CHECKPOINT).exists());
    }
}
