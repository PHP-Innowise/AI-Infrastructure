use crate::{Result, artifact};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Instant,
};
fn uuid(v: &Value) -> bool {
    v.as_str()
        .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok_and(|u| u.to_string() == s))
}
fn number(v: &Value) -> bool {
    v.as_u64().is_some_and(|n| n <= 9007199254740991)
}
pub fn known_paths(root: &Path) -> Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    fn walk(
        root: &Path,
        dir: &Path,
        out: &mut BTreeSet<String>,
        visited: &mut usize,
    ) -> Result<()> {
        if !dir.is_dir() {
            return Ok(());
        }
        for e in std::fs::read_dir(dir)? {
            *visited += 1;
            if *visited > 100000 {
                return Ok(());
            }
            let p = e?.path();
            let m = std::fs::symlink_metadata(&p)?;
            if infra_core::fs::linked(&m) {
                continue;
            }
            let n = p.file_name().unwrap_or_default().to_string_lossy();
            if m.is_dir() {
                if ![
                    ".git",
                    "local",
                    ".context-handoff",
                    "node_modules",
                    "vendor",
                    "__pycache__",
                ]
                .contains(&n.as_ref())
                {
                    walk(root, &p, out, visited)?;
                }
            } else if m.is_file() {
                out.insert(
                    p.strip_prefix(root)
                        .map_err(|_| artifact::invalid("known-path root"))?
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
            if out.len() >= 100000 {
                return Ok(());
            }
        }
        Ok(())
    }
    walk(root, root, &mut out, &mut 0)?;
    Ok(out)
}
struct Reads<'a> {
    root: &'a Path,
    remaining: usize,
    deadline: Option<Instant>,
    exhausted: bool,
}
impl Reads<'_> {
    fn read(&mut self, name: &str) -> Result<Vec<u8>> {
        artifact::relative(name)?;
        if self.deadline.is_some_and(|d| Instant::now() >= d) || self.remaining == 0 {
            self.exhausted = true;
            return Err(artifact::invalid("collection deadline or read budget"));
        }
        let limit = self.remaining.min(1024 * 1024);
        let (raw, _) = artifact::read_file(&self.root.join(name), limit).inspect_err(|_| {
            if std::fs::metadata(self.root.join(name)).is_ok_and(|m| m.len() > limit as u64) {
                self.exhausted = true;
            }
        })?;
        self.remaining = self.remaining.saturating_sub(raw.len());
        Ok(raw)
    }
}
fn checked_path(value: &Value, known: &BTreeSet<String>) -> Result<String> {
    let s = value
        .as_str()
        .filter(|s| {
            s.chars().count() <= 512
                && known.contains(*s)
                && s.chars()
                    .all(|c| c.is_alphanumeric() || "-_./ ".contains(c))
        })
        .ok_or_else(|| artifact::invalid("unrecognized path"))?;
    artifact::relative(s)?;
    artifact::public_text(s)?;
    if [
        "ignore previous",
        "system prompt",
        "you must",
        "run command",
    ]
    .iter()
    .any(|p| s.to_lowercase().contains(p))
    {
        return Err(artifact::invalid("instruction-like path"));
    }
    Ok(s.into())
}
fn render(reads: &mut Reads<'_>, manifest: &Value, selected: &Value, source: &str) -> Value {
    let r = (|| -> Result<Value> {
        let id = manifest["id"]
            .as_str()
            .ok_or_else(|| artifact::invalid("trace ID"))?;
        let trace = artifact::strict(
            &reads.read(&format!("memory-bank/local/retrieval-traces/{id}.json"))?,
        )?;
        let r = &trace["render"];
        if trace["id"] != manifest["id"]
            || trace["schema_version"] != 1
            || trace["truncated"] != false
            || !["printed_capsule", "capsule_text"].contains(&r["form"].as_str().unwrap_or(""))
            || !r["sha256"].as_str().is_some_and(|s| artifact::hex(s, 64))
            || !number(&r["bytes"])
            || r["bytes"].as_u64().unwrap() > 1048576
        {
            return Err(artifact::invalid("unknown trace"));
        }
        let images = trace["records"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| e["path"] == selected["path"])
            .collect::<Vec<_>>();
        let rows = r["records"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| e["path"] == selected["path"])
            .collect::<Vec<_>>();
        if images.len() != 1 || rows.len() != 1 || images[0]["source_sha256"] != source {
            return Err(artifact::invalid("unknown trace source"));
        }
        let row = rows[0];
        if row["header_present"] != true
            || !number(&row["body_bytes"])
            || row["body_bytes"].as_u64().unwrap() > 1048576
            || !row["pointer_only"].is_boolean()
            || !row["body_sha256"]
                .as_str()
                .is_some_and(|s| artifact::hex(s, 64))
        {
            return Err(artifact::invalid("unknown trace body"));
        }
        Ok(
            json!({"status":"trace-reported","frame_sha256":r["sha256"],"frame_bytes":r["bytes"],"form":r["form"],"body_sha256":row["body_sha256"],"body_bytes":row["body_bytes"],"pointer_only":row["pointer_only"]}),
        )
    })();
    r.unwrap_or_else(|_| json!({"status":"unknown"}))
}
fn document(
    reads: &mut Reads<'_>,
    item: &Value,
    manifest: &Value,
    known: &BTreeSet<String>,
) -> Result<Value> {
    let path = checked_path(&item["path"], known)?;
    let expected = item["source_hash"]
        .as_str()
        .filter(|s| artifact::hex(s, 64))
        .ok_or_else(|| artifact::invalid("document digest"))?;
    let mut out = json!({"document_id":artifact::sha(path.as_bytes()),"path":path,"document_sha256":expected,"document_state":"unknown","record_id":null,"revision":null,"sources":null,"render":{"status":"unknown"}});
    let observe = (|| -> Result<()> {
        let raw = reads.read(&path)?;
        let text = std::str::from_utf8(&raw)
            .map_err(|_| artifact::invalid("document UTF-8"))?
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        out["document_state"] = if artifact::sha(text.as_bytes()) == expected {
            "match"
        } else {
            "changed"
        }
        .into();
        if out["document_state"] != "match" {
            return Ok(());
        }
        out["render"] = render(reads, manifest, item, expected);
        let Some(raw) = text
            .strip_prefix("---\n")
            .and_then(|s| s.split_once("\n---").map(|p| p.0))
        else {
            return Ok(());
        };
        let meta = artifact::strict(raw.as_bytes())?;
        let id = &meta["id"];
        if (uuid(id)
            || id.as_str().is_some_and(|s| {
                s.starts_with("MEM-")
                    && s.len() <= 124
                    && s[4..]
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-')
            }))
            && artifact::public_text(id.as_str().unwrap_or("")).is_ok()
        {
            out["record_id"] = id.clone();
        }
        if number(&meta["revision"]) {
            out["revision"] = meta["revision"].clone();
        }
        let entries = meta
            .get("source_fingerprints")
            .or_else(|| meta.get("source_digests"))
            .and_then(Value::as_array)
            .filter(|e| e.len() <= 16)
            .ok_or_else(|| artifact::invalid("unknown sources"))?;
        let mut refs = vec![];
        let mut seen = BTreeSet::new();
        for e in entries {
            let name = checked_path(&e["path"], known)?;
            let expected = e["sha256"]
                .as_str()
                .filter(|s| artifact::hex(s, 64))
                .ok_or_else(|| artifact::invalid("source digest"))?;
            if !seen.insert(name.clone()) {
                return Err(artifact::invalid("duplicate source"));
            }
            let mut r = json!({"path":name,"expected_sha256":expected,"observed_sha256":null,"state":"unknown"});
            match reads.read(&name) {
                Ok(raw) => {
                    let raw = if raw.iter().take(8192).any(|b| *b == 0) {
                        raw
                    } else {
                        let mut normalized = vec![];
                        let mut i = 0;
                        while i < raw.len() {
                            if raw[i] == b'\r' && raw.get(i + 1) == Some(&b'\n') {
                                i += 1;
                            }
                            normalized.push(raw[i]);
                            i += 1;
                        }
                        normalized
                    };
                    let h = artifact::sha(&raw);
                    r["observed_sha256"] = h.clone().into();
                    r["state"] = if h == expected { "match" } else { "changed" }.into();
                }
                Err(_) => {
                    if !reads.root.join(&name).exists() {
                        r["state"] = "missing".into();
                    }
                }
            }
            refs.push(r);
        }
        if let Some(declared) = meta.get("sources") {
            let entries = declared
                .as_array()
                .filter(|e| e.len() <= 16)
                .ok_or_else(|| artifact::invalid("source list"))?;
            let names = entries
                .iter()
                .map(|e| checked_path(e, known))
                .collect::<Result<BTreeSet<_>>>()?;
            if names.len() != entries.len() || names != seen {
                return Err(artifact::invalid("source membership"));
            }
        }
        out["sources"] = json!(refs);
        Ok(())
    })();
    if observe.is_err() && !reads.root.join(&path).exists() {
        out["document_state"] = "missing".into();
    }
    Ok(out)
}
pub fn collect(
    root: &Path,
    manifests: &[PathBuf],
    known: &BTreeSet<String>,
    deadline: Option<Instant>,
) -> Value {
    let mut out = json!({"schema":"memory-delivery-proof-v1","status":"unknown","phase":"post-run","producer_authenticated":false,"host_receipt":"unknown","model_use":"unproven","semantic_verification":"unproven","frames":[],"incomplete":manifests.len()>32});
    let mut reads = Reads {
        root,
        remaining: 8 * 1024 * 1024,
        deadline,
        exhausted: false,
    };
    let mut total = 0;
    let mut paths = manifests.to_vec();
    paths.sort();
    for path in paths.into_iter().take(32) {
        let frame = (|| -> Result<Value> {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| artifact::invalid("manifest outside corpus"))?;
            if !relative.parent().is_some_and(|p| {
                p == Path::new("memory-bank/local/retrieval-manifests")
                    || p == Path::new("project-brain/control/retrieval-manifests")
            }) {
                return Err(artifact::invalid("manifest directory"));
            }
            let name = relative.to_string_lossy().replace('\\', "/");
            let raw = reads.read(&name)?;
            let m = artifact::strict(&raw)?;
            if m["schema_version"] != 4
                || !uuid(&m["id"])
                || path.file_stem().and_then(|s| s.to_str()) != m["id"].as_str()
                || !["refresh", "hook-context", "retrieve", "context"]
                    .contains(&m["entry_point"].as_str().unwrap_or(""))
                || !m["task_id"].is_null() && !uuid(&m["task_id"])
                || !m["task_revision"].is_null() && !number(&m["task_revision"])
            {
                return Err(artifact::invalid("manifest shape"));
            }
            let selected = m["selected"]
                .as_array()
                .filter(|v| v.len() <= 32)
                .ok_or_else(|| artifact::invalid("document bound"))?;
            let mut documents = vec![];
            let mut seen = BTreeSet::new();
            for item in selected {
                total += 1;
                if total > 32 {
                    return Err(artifact::invalid("total document bound"));
                }
                match document(&mut reads, item, &m, known) {
                    Ok(d) => {
                        if seen.insert(d["path"].as_str().unwrap_or("").to_string()) {
                            documents.push(d);
                        } else {
                            out["incomplete"] = true.into();
                        }
                    }
                    Err(_) => out["incomplete"] = true.into(),
                }
            }
            Ok(
                json!({"manifest_id":m["id"],"manifest_path":name,"manifest_sha256":artifact::sha(&raw),"task_uuid":m["task_id"],"task_revision":m["task_revision"],"entry_point":m["entry_point"],"origin":if ["refresh","hook-context"].contains(&m["entry_point"].as_str().unwrap_or("")){"automatic"}else{"explicit"},"selected":documents}),
            )
        })();
        match frame {
            Ok(frame) => {
                out["frames"].as_array_mut().unwrap().push(frame);
                if out.to_string().len() > 65536 {
                    out["frames"].as_array_mut().unwrap().pop();
                    out["incomplete"] = true.into();
                    break;
                }
            }
            Err(_) => out["incomplete"] = true.into(),
        }
    }
    if reads.exhausted {
        out["incomplete"] = true.into();
    }
    if !out["frames"].as_array().unwrap().is_empty() {
        out["status"] = if out["incomplete"] == true {
            "partial"
        } else {
            "observed"
        }
        .into();
    }
    out
}
