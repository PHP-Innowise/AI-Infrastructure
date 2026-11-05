use super::{
    BankService, codec,
    publication::{IMAGE_LIMIT, PublicationFiles},
};
use crate::{
    Result, RuntimeError,
    domain::{self, json},
    fs,
};
use serde_json::{Value, json as object};
use std::{path::Path, time::Duration};

const README: &str = "# Context Bundle\n\nA point-in-time, privacy-filtered copy of one installation's Project Brain\nrecords and Memory Bank chunks. `MANIFEST.json` is the index: it lists every\nitem included, and every item excluded together with the reason.\n\nThis is a handoff artifact, not a second source of truth.\n\n- **Records here are not authoritative.** Each one cites the sources it was\n  derived from. Open the cited source before acting on a claim.\n- **Chunks tagged `auto-promoted` were never read by a human.** Automatic\n  promotion is the shipped default; see the origin repository's\n  `docs/SECURITY.md`.\n- **You are not an authorized owner.** Every Project Brain record carries the\n  originating owner in `owner`/`authorized_owners`. A record copied into your\n  installation cannot be mutated by you until it is re-created under your own\n  identity. Treat this bundle as read-only history and start your own records\n  for active work.\n- **Revisions are frozen.** `revision` reflects the source installation at\n  export time. It says nothing about what happened there afterwards.\n";

pub fn export(
    service: &BankService,
    destination: &Path,
    include_archive: bool,
    include_superseded: bool,
    force: bool,
) -> Result<Value> {
    match std::fs::symlink_metadata(destination) {
        Ok(metadata) => {
            if fs::linked(&metadata) || !metadata.is_dir() {
                return Err(RuntimeError::policy(
                    "Export destination must be an ordinary directory",
                ));
            }
            if !force && std::fs::read_dir(destination)?.next().is_some() {
                return Err(RuntimeError::conflict(
                    "Export destination is not empty; explicitly select force to overwrite",
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let config = service.config()?;
    let mut privacy = config.strings("allowed_privacy")?;
    privacy.sort();
    let mut included = Vec::new();
    let mut excluded = Vec::new();
    let mut payload = Vec::new();
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    for image in service.brain()?.records(include_archive)? {
        let path = image
            .path
            .strip_prefix(&service.context.roots.state)
            .ok()
            .and_then(|path| path.to_str())
            .map(|s| s.replace('\\', "/"))
            .ok_or_else(|| RuntimeError::policy("Export source path escapes state"))?;
        let record = &image.record;
        if !privacy
            .iter()
            .any(|privacy| record["privacy"].as_str() == Some(privacy))
        {
            excluded.push(object!({"path":path,"id":record["id"],"reason":format!("privacy '{}' is not in allowed_privacy",record["privacy"].as_str().unwrap_or(""))}));
            continue;
        }
        let raw = files
            .read(&path, 4 * IMAGE_LIMIT)?
            .ok_or_else(|| RuntimeError::unavailable("Export source disappeared"))?;
        included.push(object!({"kind":"brain-record","path":path,"id":record["id"],"external_id":record["external_id"],"type":record["type"],"status":record["status"],"privacy":record["privacy"],"authority":record["authority"],"owner":record["owner"],"revision":record["revision"],"sources":record["sources"]}));
        payload.push((path, raw));
    }
    for path in codec::paths(service)? {
        let chunk = match codec::read(&mut files, &path) {
            Ok(chunk) => chunk,
            Err(error) => {
                excluded.push(
                    object!({"path":path,"id":null,"reason":format!("unreadable chunk: {error}")}),
                );
                continue;
            }
        };
        let m = &chunk.metadata;
        let status = m["status"].as_str().unwrap_or("");
        if !include_superseded && !["active", "needs-review"].contains(&status) {
            excluded.push(object!({"path":path,"id":m["id"],"reason":format!("status '{status}' is not exported without include-superseded")}));
            continue;
        }
        included.push(object!({"kind":"memory-chunk","path":path,"id":m["id"],"title":m["title"],"type":m["type"],"status":m["status"],"tags":m.get("tags").cloned().unwrap_or(object!([])),"last_verified":m["last_verified"],"sources":m.get("sources").cloned().unwrap_or(object!([])),"auto_promoted":m["tags"].as_array().is_some_and(|tags|tags.iter().any(|tag|tag=="auto-promoted"))}));
        payload.push((path, chunk.raw));
    }
    for (path, raw) in &payload {
        let text = std::str::from_utf8(raw)
            .map_err(|_| RuntimeError::invalid(format!("Export source {path} is not UTF-8")))?;
        if let Some(label) = domain::secret_label(text)? {
            return Err(RuntimeError::policy(format!(
                "Export aborted: {path} matches a {label} secret pattern; sanitize the source first"
            )));
        }
    }
    fs::ensure_directory(destination, false)?;
    for (path, raw) in payload {
        let target = destination.join(&path);
        fs::admit_descendant(destination, &target)?;
        fs::atomic_write(&target, &raw)?;
    }
    let commit = crate::git::run(
        &service.context.roots.project,
        &crate::git::strings(&["rev-parse", "HEAD"]),
        None,
        Duration::from_secs(5),
        16384,
    )
    .ok()
    .filter(|output| output.code == 0)
    .and_then(|output| String::from_utf8(output.stdout).ok())
    .map(|text| text.trim().to_string());
    let brain_count = included
        .iter()
        .filter(|item| item["kind"] == "brain-record")
        .count();
    let chunk_count = included
        .iter()
        .filter(|item| item["kind"] == "memory-chunk")
        .count();
    let automatic = included
        .iter()
        .filter(|item| item["auto_promoted"] == true)
        .count();
    let manifest = object!({"schema_version":1,"generated_at":domain::clock::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),"source":{"framework":config.value["framework"],"mode":config.value["mode"],"commit":commit,"automatic_promotion":config.value["automatic_promotion"]},"filters":{"allowed_privacy":privacy,"include_archive":include_archive,"include_superseded":include_superseded,"exported_chunk_statuses":if include_superseded{object!("all")}else{object!(["active","needs-review"])}},"counts":{"brain_records":brain_count,"memory_chunks":chunk_count,"excluded":excluded.len()},"included":included,"excluded":excluded});
    fs::atomic_write(
        &destination.join("MANIFEST.json"),
        format!(
            "{}\n",
            json::serialize(&manifest, false, false, true, true)?
        )
        .as_bytes(),
    )?;
    fs::atomic_write(&destination.join("README.md"), README.as_bytes())?;
    // The folder written, wherever the command was given from.
    let written = crate::fs::canonicalize(destination).unwrap_or_else(|_| destination.into());
    Ok(
        object!({"destination":written.to_string_lossy(),"brain_records":brain_count,"memory_chunks":chunk_count,"excluded":manifest["counts"]["excluded"],"auto_promoted_chunks":automatic}),
    )
}
