use crate::executable;
use crate::{
    Result, admission, artifact, delivery, isolation, outcome,
    process::{self, Budget, CommandSpec},
    reconstruction::{self, Reconstructor},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
pub const FORMAT: &str = "memory-native-ab-run-v1";
pub fn load_config(path: &Path) -> Result<Value> {
    let v = artifact::read_json(path, 1048576)?;
    if !artifact::exact(
        &v,
        &[
            "schema",
            "project_snapshot",
            "source",
            "served",
            "model",
            "tool_cap",
            "deadline_seconds",
        ],
    ) || v["schema"] != "memory-native-ab-v1"
    {
        return Err(artifact::invalid("invalid native experiment configuration"));
    }
    for role in ["source", "served"] {
        let task = &v[role];
        if !artifact::exact(task, &["task_alias", "prompt"]) {
            return Err(artifact::invalid("invalid native task"));
        }
        let alias = task["task_alias"]
            .as_str()
            .ok_or_else(|| artifact::invalid("task alias required"))?;
        if alias.is_empty()
            || alias.len() > 80
            || !alias.bytes().enumerate().all(|(i, b)| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || i > 0 && b"_/-".contains(&b)
            })
        {
            return Err(artifact::invalid("invalid task alias"));
        }
        let prompt = task["prompt"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 16000 && !s.contains('\0'))
            .ok_or_else(|| artifact::invalid("invalid prompt"))?;
        artifact::public_text(prompt)?;
    }
    if v["source"]["task_alias"] == v["served"]["task_alias"]
        || !v["model"].as_str().is_some_and(|s| {
            !s.is_empty()
                && s.len() <= 80
                && s.bytes()
                    .enumerate()
                    .all(|(i, b)| b.is_ascii_alphanumeric() || i > 0 && b"_.-".contains(&b))
        })
        || !v["tool_cap"]
            .as_u64()
            .is_some_and(|n| (1..=100).contains(&n))
        || !v["deadline_seconds"]
            .as_u64()
            .is_some_and(|n| (1..=900).contains(&n))
        || !v["project_snapshot"]
            .as_str()
            .is_some_and(|s| Path::new(s).is_absolute())
    {
        return Err(artifact::invalid(
            "invalid native configuration budget or identity",
        ));
    }
    Ok(v)
}
pub fn runtime_digest(reconstructor: &Reconstructor) -> Result<String> {
    let canonical = admission::canonical(&reconstructor.installer)?;
    artifact::spaced_digest(
        &json!([
            canonical
                .iter()
                .map(|(n, (raw, m))| json!([n, artifact::sha(raw), m]))
                .collect::<Vec<_>>(),
            artifact::sha(include_bytes!("native.rs")),
            artifact::sha(include_bytes!("admission.rs")),
            artifact::sha(include_bytes!("delivery.rs")),
            executable::digest(&reconstructor.runtime)?
        ]),
        true,
        false,
    )
}
pub fn summary(state: &Value) -> Result<Value> {
    let stages = state["stages"]
        .as_object()
        .ok_or_else(|| artifact::invalid("invalid native stage state"))?;
    if stages.len() != 2 || !stages.contains_key("source") || !stages.contains_key("served") {
        return Err(artifact::invalid("native stages required"));
    }
    for s in stages.values() {
        if !s["attempted"].is_boolean()
            || ![
                "planned",
                "running",
                "captured",
                "consulted",
                "not-observed",
                "failed",
                "interrupted",
                "blocked-source",
                "no-learning",
                "pending-source",
            ]
            .contains(&s["status"].as_str().unwrap_or(""))
        {
            return Err(artifact::invalid("invalid stage summary"));
        }
    }
    Ok(
        json!({"schema":FORMAT,"cohort_role":"development-smoke","planned":2,"attempted":stages.values().filter(|s|s["attempted"]==true).count(),"stages":stages.iter().map(|(k,s)|(k.clone(),s["status"].clone())).collect::<BTreeMap<_,_>>(),"causal_benefit":null,"provider_cost_savings":null,"natural_delayed_efficacy":false}),
    )
}
pub fn plan(config_path: &Path, out: &Path, reconstructor: &Reconstructor) -> Result<Value> {
    let config = load_config(config_path)?;
    let source = artifact::ordinary(Path::new(config["project_snapshot"].as_str().unwrap()))?;
    let config_path = artifact::ordinary(config_path)?;
    let out = artifact::ordinary(out)?;
    if out.exists()
        || !out.parent().is_some_and(Path::is_dir)
        || artifact::overlaps(&source, &reconstructor.source)?
        || artifact::overlaps(&out, &reconstructor.source)?
        || artifact::overlaps(&source, &out)?
        || artifact::overlaps(&source, &config_path)?
        || artifact::overlaps(&out, &config_path)?
    {
        return Err(artifact::invalid(
            "fresh private output and separated snapshot/config required",
        ));
    }
    let canon = admission::canonical(&reconstructor.installer)?;
    let image =
        admission::admitted_image(&source, artifact::ImageKind::NativePublic, &canon, None)?;
    let admitted = admission::admit(&image, &canon, false, None)?;
    let seal = json!({"schema":FORMAT,"config":config,"source_sha256":artifact::image_digest(&image,true)?,"source_admission":admitted,"runtime_sha256":runtime_digest(reconstructor)?,"created_at":chrono::Utc::now().timestamp_micros()as f64/1000000.0,"cohort_role":"development-smoke"});
    artifact::private_dir(&out)?;
    let control = out.join("control");
    artifact::private_dir(&control)?;
    artifact::write_json(&control.join("plan.json"), &seal)?;
    let state = json!({"schema":FORMAT,"plan_sha256":artifact::sha(&artifact::read_file(&control.join("plan.json"),artifact::MAX_TREE)?.0),"stages":{"source":{"status":"planned","attempted":false},"served":{"status":"planned","attempted":false}}});
    artifact::write_json(&control.join("state.json"), &state)?;
    summary(&state)
}
pub fn load_run(root: &Path) -> Result<(PathBuf, Value, Value)> {
    let root = artifact::ordinary(root)?;
    let plan = artifact::read_json(&root.join("control/plan.json"), 1048576)?;
    let state = artifact::read_json(&root.join("control/state.json"), artifact::MAX_TREE)?;
    if plan["schema"] != FORMAT
        || state["schema"] != FORMAT
        || state["plan_sha256"]
            != artifact::sha(
                &artifact::read_file(&root.join("control/plan.json"), artifact::MAX_TREE)?.0,
            )
    {
        return Err(artifact::invalid("invalid run state"));
    }
    summary(&state)?;
    Ok((root, plan, state))
}
fn metadata(root: &Path, name: &str) -> Result<Value> {
    let (raw, _) = artifact::read_file(&root.join(name), artifact::MAX_FILE)?;
    let text = std::str::from_utf8(&raw)
        .map_err(|_| artifact::invalid("record metadata UTF-8"))?
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let body = text
        .strip_prefix("---\n")
        .and_then(|s| s.split_once("\n---").map(|p| p.0))
        .ok_or_else(|| artifact::invalid("record metadata unavailable"))?;
    artifact::strict(body.as_bytes())
}
fn citations(root: &Path, meta: &Value, field: &str) -> Result<bool> {
    let rows = meta[field]
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= 16)
        .ok_or_else(|| artifact::invalid("source evidence unavailable"))?;
    let declared = meta["sources"]
        .as_array()
        .ok_or_else(|| artifact::invalid("declared sources unavailable"))?
        .iter()
        .map(|s| {
            s.as_str()
                .map(|s| s.split('#').next().unwrap_or(s).to_string())
                .ok_or_else(|| artifact::invalid("source path"))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    let mut seen = BTreeMap::new();
    for row in rows {
        let name = row["path"]
            .as_str()
            .ok_or_else(|| artifact::invalid("source path"))?;
        artifact::relative(name)?;
        let expected = row["sha256"]
            .as_str()
            .filter(|h| artifact::hex(h, 64))
            .ok_or_else(|| artifact::invalid("source hash"))?;
        if seen.get(name).is_some_and(|prior| *prior != expected) {
            return Ok(false);
        }
        seen.insert(name.to_string(), expected.to_string());
        let raw = artifact::read_file(&root.join(name), artifact::MAX_FILE)?.0;
        let normalized = if raw.iter().take(8192).any(|b| *b == 0) {
            raw
        } else {
            String::from_utf8_lossy(&raw)
                .replace("\r\n", "\n")
                .into_bytes()
        };
        if artifact::sha(&normalized) != expected {
            return Ok(false);
        }
    }
    Ok(seen.keys().cloned().collect::<BTreeSet<_>>() == declared)
}
// Native command evidence is literal argv, with one optional exact-corpus cd wrapper.
fn native_literal_command(script: &str, grouped: bool) -> bool {
    let bytes = script.as_bytes();
    let (mut at, mut quote, mut joins) = (0usize, 0u8, 0usize);
    while at < bytes.len() {
        let byte = bytes[at];
        if quote == b'\'' {
            if byte == b'\'' {
                quote = 0;
            }
            at += 1;
            continue;
        }
        if byte == b'\\' {
            at += 2;
            if at > bytes.len() {
                return false;
            }
            continue;
        }
        if quote == b'"' {
            if byte == b'"' {
                quote = 0;
            } else if byte == 0x60 {
                return false;
            } else if byte == b'$' {
                if !bytes[at..].starts_with(b"$PWD\"") {
                    return false;
                }
                at += 3;
            }
            at += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = byte,
            b'&' if grouped && joins == 0 && bytes.get(at + 1) == Some(&b'&') => {
                joins += 1;
                at += 1;
            }
            b';' | b'|' | b'&' | b'<' | b'>' | b'(' | b')' | b'$' | b'*' | b'?' | b'[' | b']'
            | b'{' | b'}' | b'~' | b'#' | b'\n' | b'\r' | 0x60 => return false,
            0..=8 | 11..=12 | 14..=31 | 127 => return false,
            _ => {}
        }
        at += 1;
    }
    quote == 0 && joins == usize::from(grouped)
}
fn admitted_native_binary(binary: &str, root: &Path, runtime: &Path, grouped: bool) -> bool {
    if !root.is_absolute() || !runtime.is_absolute() {
        return false;
    }
    for spelling in [
        "memory-bank/bin/infra-engine",
        "memory-bank/bin/infra-engine.exe",
    ] {
        let installed = root.join(spelling);
        let rooted_relative = grouped && (binary == spelling || binary == format!("./{spelling}"));
        if binary != installed.to_string_lossy() && !rooted_relative {
            continue;
        }
        // Corpus-installed bytes retain strict one-link admission. Only the
        // explicitly selected runtime uses native source capture for Cargo hardlinks.
        let Ok((installed_bytes, _)) = artifact::read_file(&installed, 512 * 1024 * 1024) else {
            return false;
        };
        let Ok((runtime_bytes, _)) = executable::read(runtime) else {
            return false;
        };
        return installed_bytes.len() == runtime_bytes.len()
            && artifact::sha(&installed_bytes) == artifact::sha(&runtime_bytes);
    }
    false
}
pub fn command_args(command: &str, root: &Path, runtime: &Path) -> Option<Vec<String>> {
    let outer = shell_words::split(command).ok()?;
    let mut script = if outer.len() == 3
        && [
            "bash",
            "sh",
            "/bin/bash",
            "/usr/bin/bash",
            "/bin/sh",
            "/usr/bin/sh",
        ]
        .contains(&outer[0].as_str())
        && ["-c", "-lc", "-ic", "-lic"].contains(&outer[1].as_str())
    {
        outer[2].as_str()
    } else {
        command
    };
    script = script.trim();
    let grouped = script.starts_with('(') && script.ends_with(')');
    if grouped {
        script = &script[1..script.len() - 1];
    }
    let mut words = shell_words::split(script).ok()?;
    if grouped {
        if words.len() < 3
            || words[0] != "cd"
            || words[1] != root.to_string_lossy()
            || words[2] != "&&"
        {
            return None;
        }
        words.drain(..3);
    }
    let binary = words.first()?;
    let selected = Path::new(binary).is_absolute() && binary == &runtime.to_string_lossy();
    let historical_alias = binary == "infra-cli" || binary == "infra-runtime";
    let installed =
        !selected && !historical_alias && admitted_native_binary(binary, root, runtime, grouped);
    if !selected && !historical_alias && !installed {
        return None;
    }
    words.remove(0);
    let native_dispatch = words.first().map(String::as_str) == Some("memory");
    if installed && !native_dispatch {
        return None;
    }
    if native_dispatch {
        if !native_literal_command(script, grouped) {
            return None;
        }
        words.remove(0);
    }
    // Legacy exact-runtime and named evidence aliases retain their pre-dispatch
    // argument shape; native dispatch consumes only the literal memory prefix.
    if words.first().map(String::as_str) == Some("--root") {
        if words.len() < 2 || words[1] != root.to_string_lossy() && words[1] != "$PWD" {
            return None;
        }
        if words[1] == "$PWD" && !script.contains("--root \"$PWD\"") {
            return None;
        }
        words.drain(..2);
    }
    Some(words)
}
pub fn save_call(command: &str, root: &Path, runtime: &Path) -> Option<(String, String, Value)> {
    let outer = shell_words::split(command).ok();
    let script = outer
        .as_ref()
        .filter(|w| w.len() == 3 && ["-c", "-lc", "-ic", "-lic"].contains(&w[1].as_str()))
        .map(|w| w[2].as_str())
        .unwrap_or(command);
    let (first, body) = script.split_once('\n')?;
    let head = regex::Regex::new(r"<<[ \t]*'([A-Za-z0-9_]+)'")
        .ok()?
        .captures(first)?;
    let delimiter = head.get(1)?.as_str();
    let body = body
        .strip_suffix(&format!("\n{delimiter}"))
        .or_else(|| body.strip_suffix(&format!("\n{delimiter}\n")))?;
    let invocation = if let Some((_, tail)) = first.split_once('|') {
        tail.trim().to_string()
    } else {
        first[..head.get(0)?.start()].trim().to_string()
    };
    let args = command_args(&invocation, root, runtime)?;
    if args.len() != 8
        || args[0] != "native-result"
        || args[1] != "--request"
        || args[3] != "--token"
        || args[5..] != ["--input", "-", "--json"]
        || !artifact::hex(&args[2], 64)
        || !artifact::hex(&args[4], 32)
    {
        return None;
    }
    Some((
        args[2].clone(),
        args[4].clone(),
        artifact::strict(body.as_bytes()).ok()?,
    ))
}
pub fn capture(
    root: &Path,
    identity: &Value,
    session: &str,
    events: &[Value],
    before: &BTreeSet<String>,
    runtime: &Path,
) -> Value {
    let mut result = json!({"native_saved":false,"new_chunks":[],"record_ids":[],"producer_authenticated":false,"semantic_verification":"unproven"});
    let attempt = (|| -> Result<Value> {
        let context = infra_core::RuntimeContext::new(
            infra_core::Roots::installed(root)?,
            "local",
            "governed",
        )?;
        let service = infra_core::results::ResultService::new(context.clone());
        for event in events {
            let item = &event["item"];
            if event["type"] != "item.completed"
                || item["type"] != "command_execution"
                || item["exit_code"] != 0
            {
                continue;
            }
            let Some((key, token, payload)) = item["command"]
                .as_str()
                .and_then(|c| save_call(c, root, runtime))
            else {
                continue;
            };
            let answer =
                artifact::strict(item["aggregated_output"].as_str().unwrap_or("").as_bytes())?;
            let state = artifact::read_json(
                &root
                    .join("project-brain/local/native-results")
                    .join(format!("{key}.json")),
                1048576,
            )?;
            let project = artifact::sha(std::fs::canonicalize(root)?.to_string_lossy().as_bytes());
            let session_hash = artifact::sha(session.as_bytes());
            let expected = artifact::spaced_digest(
                &json!([
                    project,
                    state["owner"],
                    "codex",
                    session_hash,
                    identity["task_id"]
                ]),
                false,
                true,
            )?;
            if key != expected
                || state["phase"] != "saved"
                || state["host"] != "codex"
                || state["project"] != project
                || state["session"] != session_hash
                || state["task_id"] != identity["task_id"]
                || state["task_uuid"] != identity["task_uuid"]
                || state["token"] != token
                || state["owner"].is_null()
            {
                continue;
            }
            let normalized = service.normalize(&payload, false)?;
            let uuid = identity["task_uuid"]
                .as_str()
                .ok_or_else(|| artifact::invalid("task UUID"))?;
            let digest = infra_core::results::request_digest(uuid, &normalized)?;
            if state["digest"] != digest
                || answer["attestation"] != "agent"
                || answer["replayed"] != false
            {
                continue;
            }
            let result_id = format!("native-{token}");
            let receipts = infra_core::result_store::EvidenceStore::new(context.clone());
            let receipt_path = receipts.receipt_path(uuid, &result_id);
            let receipt = artifact::read_json(&receipt_path, 4096)?;
            let proof =
                artifact::read_json(&receipts.attempt_path(uuid, &result_id, &digest), 4096)?;
            if receipt["digest"] != digest
                || receipt["task_id"] != uuid
                || receipt["result_id"] != result_id
                || proof["digest"] != digest
                || proof["task_id"] != uuid
                || proof["result_id"] != result_id
                || proof["state"] != "complete"
                || proof["profile"] != "memory-result-proof-v1"
                || proof["attestation"] != "agent"
                || proof["owner"] != state["owner"]
            {
                continue;
            }
            let task = service.task(uuid, false)?.record;
            if receipt["task_revision"].is_null() {
                if !answer["task"].is_null() {
                    continue;
                }
            } else if answer["task"]["id"] != uuid
                || answer["task"]["revision"] != receipt["task_revision"]
                || task["revision"] != receipt["task_revision"]
            {
                continue;
            }
            if normalized
                .progress
                .as_ref()
                .is_some_and(|s| task["progress"] != *s)
            {
                continue;
            }
            let reported = answer["records"]
                .as_array()
                .filter(|a| a.len() == normalized.learnings.len())
                .ok_or_else(|| artifact::invalid("learning report mismatch"))?;
            let mut chunks = BTreeSet::new();
            let mut ids = vec![];
            for (row, learning) in reported.iter().zip(&normalized.learnings) {
                let id = row["id"]
                    .as_str()
                    .filter(|s| uuid::Uuid::parse_str(s).is_ok())
                    .ok_or_else(|| artifact::invalid("record ID"))?;
                let name = format!(
                    "project-brain/dynamic/{}/{id}.md",
                    if learning.kind == "finding" {
                        "findings"
                    } else {
                        "decisions"
                    }
                );
                let record = metadata(root, &name)?;
                let external = infra_core::results::learning_id(uuid, learning)?;
                let expected_external = if learning.record_id.is_some() {
                    record["external_id"].clone()
                } else {
                    external.into()
                };
                if learning.record_id.as_ref().is_some_and(|s| s != id)
                    || row["external_id"] != expected_external
                    || record["external_id"] != expected_external
                    || record["id"] != id
                    || record["type"] != learning.kind
                    || row["type"] != learning.kind
                    || record["title"] != learning.title
                    || record["sources"]
                        .as_array()
                        .unwrap_or(&vec![])
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<BTreeSet<_>>()
                        != learning
                            .sources
                            .iter()
                            .map(String::as_str)
                            .collect::<BTreeSet<_>>()
                    || row["status"] != record["status"]
                    || row["revision"] != record["revision"]
                    || !["created", "updated", "completed"]
                        .contains(&row["state"].as_str().unwrap_or(""))
                    || record["status"]
                        != if learning.kind == "finding" {
                            "resolved"
                        } else {
                            "accepted"
                        }
                    || infra_core::results::consequence(&record)
                        != learning
                            .consequence
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                    || !citations(root, &record, "source_fingerprints")?
                {
                    return Err(artifact::invalid("learning mismatch"));
                }
                ids.push(id.to_string());
                if row["state"] != "created" || before.contains(&name) {
                    continue;
                }
                let memory_ids = answer["promotion"]["promoted"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|p| p["record_id"] == id)
                    .filter_map(|p| p["memory_id"].as_str())
                    .collect::<BTreeSet<_>>();
                let folder = root.join("memory-bank/chunks");
                if folder.is_dir() {
                    let entries =
                        std::fs::read_dir(&folder)?.collect::<std::io::Result<Vec<_>>>()?;
                    if entries.len() > 2000 {
                        return Err(artifact::invalid("chunk scan bound"));
                    }
                    for e in entries {
                        let p = e.path();
                        let n = format!("memory-bank/chunks/{}", e.file_name().to_string_lossy());
                        if before.contains(&n) || !p.extension().is_some_and(|s| s == "md") {
                            continue;
                        }
                        let meta = metadata(root, &n)?;
                        if meta["id"]
                            .as_str()
                            .is_some_and(|id| memory_ids.contains(id))
                            && meta["sources"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .any(|s| s == &name)
                            && citations(root, &meta, "source_digests")?
                        {
                            chunks.insert(n);
                        }
                    }
                }
            }
            return Ok(
                json!({"native_saved":true,"new_chunks":chunks,"record_ids":ids,"producer_authenticated":false,"semantic_verification":"unproven","result_id":result_id,"digest":digest,"receipt":receipt_path.strip_prefix(root).map_err(|_|artifact::invalid("receipt root"))?,"receipt_sha256":artifact::sha(&artifact::read_file(&receipt_path,4096)?.0)}),
            );
        }
        Ok(result.clone())
    })();
    if let Ok(value) = attempt {
        result = value;
    }
    result
}
pub fn pending(root: &Path) -> Result<Value> {
    let database = root.join("memory-bank/local/context.db");
    let store = artifact::open_database_read_only(
        &database,
        artifact::MAX_TREE,
        Duration::from_millis(200),
    )?;
    let count = store
        .conn
        .query_row("SELECT count(*) FROM turn_deltas", [], |r| {
            r.get::<_, u64>(0)
        })?;
    let mut stmt = store
        .conn
        .prepare("SELECT DISTINCT task_id FROM turn_deltas LIMIT 33")?;
    let tasks = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let requests = root.join("project-brain/local/native-results");
    let mut unresolved = 0;
    let mut n = 0;
    if requests.is_dir() {
        for e in std::fs::read_dir(requests)? {
            let p = e?.path();
            if p.extension().is_some_and(|s| s == "json") {
                n += 1;
                if n > 32 {
                    return Err(artifact::invalid("native request bound"));
                }
                unresolved += usize::from(artifact::read_json(&p, 1048576)?["phase"] != "saved");
            }
        }
    }
    fn queued(p: &Path, n: &mut usize, q: &mut usize) -> Result<()> {
        if !p.is_dir() {
            return Ok(());
        }
        for e in std::fs::read_dir(p)? {
            let p = e?.path();
            *n += 1;
            if *n > 10000 {
                return Err(artifact::invalid("temporary queue bound"));
            }
            let m = std::fs::symlink_metadata(&p)?;
            if infra_core::fs::linked(&m) {
                return Err(artifact::invalid("linked temporary directory"));
            }
            if m.is_dir() {
                queued(&p, n, q)?;
            } else if p.components().any(|c| {
                let s = c.as_os_str().to_string_lossy();
                s.starts_with("ai-accelerator-native-spool-")
                    || s.starts_with("ai-accelerator-memory-queue-")
            }) {
                let name = p.file_name().unwrap_or_default().to_string_lossy();
                *q += usize::from(name.contains(".json") || name.contains(".attempt"));
            }
        }
        Ok(())
    }
    let mut q = 0;
    queued(
        &root.join("memory-bank/local/native-ab/tmp"),
        &mut 0,
        &mut q,
    )?;
    Ok(
        json!({"turn_deltas":count,"turn_task_ids":tasks,"unresolved_requests":unresolved,"queued_requests":q,"retained":true,"lossless_cache_deletion":false}),
    )
}
fn healthy(root: &Path) -> Result<bool> {
    let (raw, _) = artifact::read_file(
        &root.join("memory-bank/local/native-results.ndjson"),
        1048576,
    )?;
    let rows = raw
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .map(artifact::strict)
        .collect::<Result<Vec<_>>>()?;
    let exits = rows
        .iter()
        .filter(|r| r["stage"] == "stop-hook" && r["event"] == "exited")
        .collect::<Vec<_>>();
    Ok(exits.len() >= 2 && exits.iter().all(|r| r["status"] == 0))
}
const SAVE_FORMAT: &str = "\nThis is a read-only research experiment. Do not modify application files or delegate. Do not manually create, update or promote reusable findings, decisions or memory chunks before the internal Stop continuation. Do not call record-result or memory_write early; retain checked learning for the native-result continuation. Follow the native memory workflow. If an internal Stop requests native-result, invoke its exact native command directly with a single quoted heredoc <<'NATIVE_RESULT' containing your own JSON, followed by NATIVE_RESULT on a new line. Do not wrap the save in another program. Each learning consequence is at most 1000 characters. Run an explicit retrieve in its own shell call so its exact stdout can be checked against the render trace. When consulting a prior memory, cite its MEM identifier. verified=true is agent attestation.";
pub fn execute_stage(
    run_root: &Path,
    seal: &Value,
    role: &str,
    source_result: Option<&Value>,
    r: &Reconstructor,
) -> Result<Value> {
    let started = Instant::now();
    let config = &seal["config"];
    let deadline = started
        + Duration::from_secs(
            config["deadline_seconds"]
                .as_u64()
                .ok_or_else(|| artifact::invalid("stage deadline"))?,
        );
    let control = run_root.join("control");
    let work = run_root.join("work");
    artifact::private_dir(&work)?;
    let corpus = work.join(role);
    let task = &config[role];
    let input = if role == "source" {
        PathBuf::from(config["project_snapshot"].as_str().unwrap())
    } else {
        work.join("source")
    };
    let canon = admission::canonical(&r.installer)?;
    let image = admission::admitted_image(
        &input,
        artifact::ImageKind::NativePublic,
        &canon,
        Some(deadline),
    )?;
    let expected = if role == "source" {
        &seal["source_sha256"]
    } else {
        &source_result.ok_or_else(|| artifact::invalid("source result required"))?["handoff_sha256"]
    };
    if expected != &artifact::image_digest(&image, true)? {
        return Err(artifact::invalid("source/handoff image changed"));
    }
    admission::admit(&image, &canon, false, Some(deadline))?;
    artifact::copy_image(&image, &corpus)?;
    let expected_copy = artifact::Scratch::new(&control, "expected-overlay")?;
    let expected_root = expected_copy.0.join("corpus");
    artifact::copy_image(&image, &expected_root)?;
    for root in [&corpus, &expected_root] {
        r.installer.overlay(
            root,
            "PHP Core",
            &reconstruction::TOOLS
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
        )?;
        for name in ["runtime.json", "providers.json", "telemetry.json"] {
            let p = r.source.join("PHP Core/project-brain/config").join(name);
            if p.is_file() {
                let (raw, _) = artifact::read_file(&p, artifact::MAX_FILE)?;
                infra_core::fs::atomic_write(&root.join("project-brain/config").join(name), &raw)?;
                artifact::chmod(&root.join("project-brain/config").join(name), 0o644)?;
            }
        }
        for name in ["memory-bank", "project-brain", ".context-handoff"] {
            artifact::private_dir(&root.join(name))?;
        }
    }
    let expected_image = admission::admitted_image(
        &expected_root,
        artifact::ImageKind::Physical,
        &canon,
        Some(deadline),
    )?;
    let mut gate =
        admission::MaterializedGate::new(&corpus, &image, &expected_image, canon, Some(deadline))?;
    let mut env = reconstruction::runtime_env(&control, chrono::Utc::now(), false)?;
    let home = control.join(format!("{role}-prep-home"));
    let tmp = control.join(format!("{role}-prep-tmp"));
    artifact::private_dir(&home)?;
    artifact::private_dir(&tmp)?;
    env.insert("HOME".into(), home.to_string_lossy().into());
    env.insert("TMPDIR".into(), tmp.to_string_lossy().into());
    env.insert("PROJECT_BRAIN_OWNER".into(), "local".into());
    let trusted = |label: &str, args: &[String]| -> Result<Vec<u8>> {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(artifact::invalid("stage deadline"));
        }
        let mut argv = vec![
            r.runtime.to_string_lossy().into(),
            "memory".into(),
            "--root".into(),
            corpus.to_string_lossy().into(),
        ];
        argv.extend(args.iter().cloned());
        let spec = CommandSpec {
            argv,
            cwd: corpus.clone(),
            env: env.clone(),
            input: vec![],
        };
        let target = control.join(format!("{role}-{label}.stdout"));
        let budget = Budget {
            timeout: left,
            cap: 100,
            max_bytes: 1048576,
            native: false,
            strict_events: false,
            error_diagnostics_terminal: false,
        };
        let result = process::execute(
            &spec,
            &budget,
            Some(&target),
            Some(&control.join(format!("{role}-{label}.stderr"))),
            |_, _| None,
        )?;
        if !result.completed {
            return Err(artifact::invalid("trusted preparation failed"));
        }
        Ok(artifact::read_file(&target, 1048576)?.0)
    };
    for (label, args) in [
        ("bank-reindex", vec!["reindex-bank", "--json"]),
        ("bank-validate", vec!["validate-bank", "--json"]),
        ("brain-validate", vec!["brain-validate"]),
        ("index", vec!["index", "--json"]),
    ] {
        trusted(
            label,
            &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )?;
    }
    reconstruction::git_raw(
        &corpus,
        &[
            "init".into(),
            "-q".into(),
            "--initial-branch".into(),
            task["task_alias"].as_str().unwrap().into(),
        ],
        Some(&env),
    )?;
    let identity = artifact::strict(&trusted(
        "start",
        &[
            "start".into(),
            "--task-id".into(),
            task["task_alias"].as_str().unwrap().into(),
            "--goal".into(),
            "Independent native-memory research".into(),
            "--json".into(),
        ],
    )?)?;
    artifact::write_json(&control.join(format!("{role}-identity.json")), &identity)?;
    if role == "served" && identity["task_uuid"] == source_result.unwrap()["identity"]["task_uuid"]
    {
        return Err(artifact::invalid("same task UUID"));
    }
    reconstruction::git_raw(
        &corpus,
        &[
            "-c".into(),
            "core.hooksPath=/dev/null".into(),
            "add".into(),
            "--".into(),
            ".".into(),
        ],
        Some(&env),
    )?;
    reconstruction::git_raw(
        &corpus,
        &[
            "-c".into(),
            "core.hooksPath=/dev/null".into(),
            "-c".into(),
            "commit.gpgsign=false".into(),
            "commit".into(),
            "-qm".into(),
            "Screened native-memory baseline".into(),
            "--allow-empty".into(),
        ],
        Some(&env),
    )?;
    let before_image = gate.image(artifact::ImageKind::NativePublic)?;
    let before = before_image
        .iter()
        .map(|f| f.path.clone())
        .collect::<BTreeSet<_>>();
    let before_path = control.join(format!("{role}-before.json"));
    artifact::write_json(
        &before_path,
        &json!({"image_sha256":artifact::image_digest(&before_image,true)?,"files":before_image.iter().map(|f|json!({"path":f.path,"sha256":artifact::sha(&f.bytes),"mode":f.mode})).collect::<Vec<_>>()}),
    )?;
    let known = delivery::known_paths(&corpus)?;
    gate.seal_generated()?;
    let prompt = format!("{}{SAVE_FORMAT}", task["prompt"].as_str().unwrap());
    let mut command = vec![
        "codex".into(),
        "exec".into(),
        "--json".into(),
        "--ignore-user-config".into(),
        "--skip-git-repo-check".into(),
        "--dangerously-bypass-hook-trust".into(),
        "-s".into(),
        "workspace-write".into(),
        "-C".into(),
        corpus.to_string_lossy().into(),
        "-m".into(),
        config["model"].as_str().unwrap().into(),
        "-c".into(),
        "features.hooks=true".into(),
        "-c".into(),
        "features.multi_agent=false".into(),
        "-c".into(),
        "agents.enabled=false".into(),
        "-c".into(),
        "project_doc_max_bytes=131072".into(),
    ];
    let hooks = artifact::read_json(&corpus.join(".codex/hooks.json"), 1048576)?;
    for (event, groups) in hooks["hooks"]
        .as_object()
        .ok_or_else(|| artifact::invalid("native hooks required"))?
    {
        command.extend([
            "-c".into(),
            format!("hooks.{event}={}", outcome::toml(groups)?),
        ]);
    }
    command.push(prompt);
    let mut view = isolation::prepare(&corpus, &control, &command, true, None)?;
    let evidence = corpus.join("memory-bank/local/native-ab");
    for name in ["home", "tmp"] {
        artifact::private_dir(&evidence.join(name))?;
    }
    let pos = view
        .prefix
        .windows(3)
        .position(|a| a[0] == "--ro-bind" && a[2] == "/home/agent/.codex/auth.json");
    if let Some(pos) = pos {
        artifact::fresh_file(&evidence.join("home/auth.json"), b"", 0o600)?;
        view.prefix.splice(
            pos..pos,
            [
                "--bind".into(),
                evidence.join("home").to_string_lossy().into(),
                "/home/agent/.codex".into(),
            ],
        );
    } else {
        view.prefix.extend([
            "--bind".into(),
            evidence.join("home").to_string_lossy().into(),
            "/home/agent/.codex".into(),
        ]);
    }
    if let Some(pos) = view
        .prefix
        .windows(2)
        .position(|a| a[0] == "--tmpfs" && a[1] == "/tmp")
    {
        view.prefix.splice(
            pos..pos + 2,
            [
                "--bind".into(),
                evidence.join("tmp").to_string_lossy().into(),
                "/tmp".into(),
            ],
        );
    }
    for e in std::fs::read_dir(&corpus)? {
        let p = e?.path();
        let n = p.file_name().unwrap_or_default().to_string_lossy();
        if !["memory-bank", "project-brain", ".context-handoff"].contains(&n.as_ref()) {
            view.prefix.extend([
                "--ro-bind".into(),
                p.to_string_lossy().into(),
                p.to_string_lossy().into(),
            ]);
        }
    }
    for name in [
        "memory-bank/scripts",
        "memory-bank/templates",
        "project-brain/config",
        "project-brain/schemas",
        "project-brain/scripts",
        "project-brain/templates",
    ] {
        let p = corpus.join(name);
        if p.exists() {
            view.prefix.extend([
                "--ro-bind".into(),
                p.to_string_lossy().into(),
                p.to_string_lossy().into(),
            ]);
        }
    }
    view.prefix.extend([
        "--setenv".into(),
        "TMPDIR".into(),
        "/tmp".into(),
        "--setenv".into(),
        "CONTEXT_TASK_ID".into(),
        task["task_alias"].as_str().unwrap().into(),
        "--setenv".into(),
        "PROJECT_BRAIN_OWNER".into(),
        "local".into(),
        "--setenv".into(),
        "GIT_OPTIONAL_LOCKS".into(),
        "0".into(),
    ]);
    for probe in [
        control.join("plan.json"),
        PathBuf::from(config["project_snapshot"].as_str().unwrap()),
    ]
    .into_iter()
    .chain((role == "served").then(|| work.join("source")))
    {
        if !isolation::probe(&view, &probe) {
            return Err(artifact::invalid("read-isolation probe failed"));
        }
    }
    artifact::write_json(
        &control.join(format!("{role}-admission.json")),
        &gate.checkpoint()?,
    )?;
    artifact::write_json(
        &control.join(format!("{role}-admission-verified.json")),
        &gate.verify()?,
    )?;
    let events = control.join(format!("{role}-events.jsonl"));
    let session = process::execute(
        &view.spec(),
        &Budget::native(
            deadline.saturating_duration_since(Instant::now()),
            config["tool_cap"].as_u64().unwrap() as usize,
        ),
        Some(&events),
        Some(&control.join(format!("{role}-native.stderr"))),
        |_, _| None,
    )?;
    let mut result = session.value();
    result["identity"] = identity.clone();
    result["native_saved"] = false.into();
    result["new_chunks"] = json!([]);
    result["before_inventory_sha256"] =
        artifact::sha(&artifact::read_file(&before_path, artifact::MAX_TREE)?.0).into();
    artifact::write_json(&control.join(format!("{role}-process.json")), &result)?;
    if !session.completed || session.sessions.len() != 1 {
        return Ok(result);
    }
    if role == "served" && json!(session.sessions) == source_result.unwrap()["sessions"] {
        return Err(artifact::invalid("same native session"));
    }
    let application = |image: &[artifact::ImageFile]| {
        artifact::image_digest(
            &image
                .iter()
                .filter(|f| {
                    !f.path.starts_with("memory-bank/") && !f.path.starts_with("project-brain/")
                })
                .cloned()
                .collect::<Vec<_>>(),
            true,
        )
    };
    if application(&before_image)? != application(&gate.image(artifact::ImageKind::NativePublic)?)?
    {
        return Err(artifact::invalid("unexpected application image change"));
    }
    for (k, v) in capture(
        &corpus,
        &identity,
        &session.sessions[0],
        &session.events,
        &before,
        &r.runtime,
    )
    .as_object()
    .unwrap()
    {
        result[k] = v.clone();
    }
    result["hooks_healthy"] = healthy(&corpus)?.into();
    result["pending"] = pending(&corpus)?;
    if role == "source" {
        if result["native_saved"] == true
            && result["hooks_healthy"] == true
            && result["pending"]["turn_deltas"].as_u64().unwrap_or(0) > 0
            && result["pending"]["unresolved_requests"] == 0
            && result["pending"]["queued_requests"] == 0
        {
            result["pending_before_settle"] = result["pending"].clone();
            let chunks = result["new_chunks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|n| {
                    Ok((
                        n.to_string(),
                        artifact::sha(&artifact::read_file(&corpus.join(n), artifact::MAX_FILE)?.0),
                    ))
                })
                .collect::<Result<BTreeMap<_, _>>>()?;
            trusted(
                "settle",
                &[
                    "turn".into(),
                    "--flush".into(),
                    "--task-id".into(),
                    task["task_alias"].as_str().unwrap().into(),
                    "--json".into(),
                ],
            )?;
            result["pending"] = pending(&corpus)?;
            for (n, h) in chunks {
                if artifact::sha(&artifact::read_file(&corpus.join(n), artifact::MAX_FILE)?.0) != h
                {
                    return Err(artifact::invalid("knowledge changed while settling"));
                }
            }
        }
        let clear = ["turn_deltas", "unresolved_requests", "queued_requests"]
            .iter()
            .all(|k| result["pending"][*k] == 0);
        let ready = clear
            && result["native_saved"] == true
            && result["hooks_healthy"] == true
            && result["new_chunks"]
                .as_array()
                .is_some_and(|a| !a.is_empty());
        result["handoff_ready"] = ready.into();
        if ready {
            result["handoff_sha256"] =
                artifact::image_digest(&gate.image(artifact::ImageKind::NativePublic)?, true)?
                    .into();
        }
    } else {
        let mut manifests = vec![];
        for name in [
            "memory-bank/local/retrieval-manifests",
            "project-brain/control/retrieval-manifests",
        ] {
            let dir = corpus.join(name);
            if dir.is_dir() {
                for e in std::fs::read_dir(dir)? {
                    let p = e?.path();
                    if p.extension().is_some_and(|e| e == "json") {
                        manifests.push(p);
                    }
                }
            }
        }
        let proof = delivery::collect(&corpus, &manifests, &known, Some(deadline));
        let expected = source_result.unwrap()["new_chunks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<BTreeSet<_>>();
        let mut found = BTreeSet::new();
        for frame in proof["frames"].as_array().into_iter().flatten() {
            if frame["task_uuid"] != identity["task_uuid"] {
                continue;
            }
            for item in frame["selected"].as_array().into_iter().flatten() {
                if item["document_state"] == "match"
                    && item["render"]["status"] == "trace-reported"
                    && item["render"]["pointer_only"] == false
                    && let Some(p) = item["path"].as_str().filter(|p| expected.contains(*p))
                {
                    let hash = item["render"]["frame_sha256"].as_str().unwrap_or("");
                    let bytes = item["render"]["frame_bytes"].as_u64().unwrap_or(0) as usize;
                    let stdout = session
                        .events
                        .iter()
                        .filter(|e| {
                            e["type"] == "item.completed"
                                && e["item"]["exit_code"] == 0
                                && e["item"]["command"]
                                    .as_str()
                                    .and_then(|s| command_args(s, &corpus, &r.runtime))
                                    .is_some_and(|a| {
                                        a.first().is_some_and(|s| s == "retrieve" || s == "context")
                                    })
                        })
                        .filter_map(|e| e["item"]["aggregated_output"].as_str());
                    if stdout
                        .into_iter()
                        .any(|s| s.len() >= bytes && artifact::sha(&s.as_bytes()[..bytes]) == hash)
                    {
                        found.insert(p.to_string());
                    }
                }
            }
        }
        artifact::write_json(&control.join("served-delivery.json"), &proof)?;
        result["attribution_observed"] = found
            .iter()
            .any(|p| {
                metadata(&corpus, p)
                    .ok()
                    .and_then(|m| m["id"].as_str().map(String::from))
                    .is_some_and(|id| session.answer.contains(&id))
            })
            .into();
        result["delivered_chunks"] = json!(found);
    }
    result["seconds"] = json!(started.elapsed().as_secs_f64());
    if Instant::now() >= deadline {
        return Err(artifact::invalid("stage deadline"));
    }
    Ok(result)
}
pub fn run(root: &Path, stage: &str, r: &Reconstructor) -> Result<Value> {
    if !["both", "source", "served"].contains(&stage) {
        return Err(artifact::invalid("invalid stage"));
    }
    let (root, seal, mut state) = load_run(root)?;
    let _lock = artifact::ExclusiveLock::acquire(&root.join("run.lock"))?;
    let roles = if stage == "both" {
        vec!["source", "served"]
    } else {
        vec![stage]
    };
    for role in &roles {
        if state["stages"][*role]["attempted"] == true {
            return Err(artifact::invalid(
                "implicit retry refused; create a new plan",
            ));
        }
    }
    for role in roles {
        if crate::cancel::interrupted() {
            break;
        }
        if role == "served" && state["stages"]["source"]["status"] != "captured" {
            state["stages"]["served"]["status"] = "blocked-source".into();
            break;
        }
        state["stages"][role]["status"] = "running".into();
        state["stages"][role]["attempted"] = true.into();
        state["stages"][role]["started_at"] =
            json!(chrono::Utc::now().timestamp_micros() as f64 / 1000000.0);
        artifact::write_json(&root.join("control/state.json"), &state)?;
        let source = state["stages"]["source"]["result"].clone();
        let result = if runtime_digest(r)? != seal["runtime_sha256"] {
            Err(artifact::invalid("runtime changed after plan"))
        } else {
            execute_stage(&root, &seal, role, (role == "served").then_some(&source), r)
        };
        match result {
            Ok(result) => {
                let status = if crate::cancel::interrupted() || result["stopped"] == "interrupted" {
                    "interrupted"
                } else if result["completed"] != true
                    || result["native_saved"] != true
                    || result["hooks_healthy"] == false
                {
                    "failed"
                } else if role == "source" {
                    if result["handoff_ready"] == true {
                        "captured"
                    } else if result["new_chunks"].as_array().is_none_or(Vec::is_empty) {
                        "no-learning"
                    } else {
                        "pending-source"
                    }
                } else if result["delivered_chunks"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty())
                    && result["attribution_observed"] == true
                {
                    "consulted"
                } else {
                    "not-observed"
                };
                state["stages"][role]["status"] = status.into();
                state["stages"][role]["result"] = result;
            }
            Err(_) => {
                state["stages"][role]["status"] = "failed".into();
                state["stages"][role]["failure"] = "stage-unavailable".into();
                let path = root.join(format!("control/{role}-process.json"));
                if path.is_file() {
                    state["stages"][role]["result"] =
                        artifact::read_json(&path, artifact::MAX_TREE)?;
                }
            }
        }
        if role == "source" && state["stages"]["source"]["status"] != "captured" {
            state["stages"]["served"]["status"] = "blocked-source".into();
        }
        artifact::write_json(&root.join("control/state.json"), &state)?;
        if role == "source" && state["stages"]["source"]["status"] != "captured" {
            break;
        }
    }
    artifact::write_json(&root.join("control/state.json"), &state)?;
    summary(&state)
}

#[cfg(test)]
mod native_command_evidence_tests {
    use super::*;
    struct Fixture {
        directory: PathBuf,
        root: PathBuf,
        runtime: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let directory = std::env::temp_dir()
                .join(format!("infra-native-evidence-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&directory).unwrap();
            let directory = std::fs::canonicalize(directory).unwrap();
            let root = directory.join("corpus with spaces");
            std::fs::create_dir_all(root.join("memory-bank/bin")).unwrap();
            let runtime = directory.join("selected engine");
            let actual = std::env::current_exe().unwrap();
            std::fs::copy(&actual, &runtime).unwrap();
            std::fs::copy(&actual, root.join("memory-bank/bin/infra-engine")).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                for image in [&runtime, &root.join("memory-bank/bin/infra-engine")] {
                    std::fs::set_permissions(image, std::fs::Permissions::from_mode(0o700))
                        .unwrap();
                }
            }
            Self {
                directory,
                root,
                runtime,
            }
        }
        fn installed(&self) -> PathBuf {
            self.root.join("memory-bank/bin/infra-engine")
        }
        fn args(&self, command: &str) -> Option<Vec<String>> {
            command_args(command, &self.root, &self.runtime)
        }
        fn command(&self, executable: &Path, operation: &str) -> String {
            format!(
                "{} memory --root {} {operation}",
                shell_words::quote(&executable.to_string_lossy()),
                shell_words::quote(&self.root.to_string_lossy())
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
    #[test]
    fn native_dispatch_normalizes_the_selected_runtime_and_installed_image() {
        let f = Fixture::new();
        for executable in [&f.runtime, &f.installed()] {
            let command = f.command(executable, "retrieve 'invoice; $(literal)' --json");
            assert_eq!(
                f.args(&command).unwrap(),
                vec!["retrieve", "invoice; $(literal)", "--json"]
            );
        }
    }
    #[test]
    fn corpus_engine_requires_matching_bytes_and_an_explicit_native_dispatch() {
        let f = Fixture::new();
        assert!(
            f.args(&f.command(&f.installed(), "retrieve invoice"))
                .is_some()
        );
        assert!(
            f.args(&format!(
                "{} --root {} retrieve invoice",
                shell_words::quote(&f.installed().to_string_lossy()),
                shell_words::quote(&f.root.to_string_lossy())
            ))
            .is_none()
        );
        std::fs::write(f.installed(), b"a different executable").unwrap();
        assert!(
            f.args(&f.command(&f.installed(), "retrieve invoice"))
                .is_none()
        );
    }
    #[test]
    fn native_relative_binary_is_allowed_only_inside_the_exact_corpus_wrapper() {
        let f = Fixture::new();
        let invocation = "memory-bank/bin/infra-engine memory --root \"$PWD\" retrieve invoice";
        assert!(f.args(invocation).is_none());
        let wrapped = format!(
            "(cd {} && {invocation})",
            shell_words::quote(&f.root.to_string_lossy())
        );
        assert_eq!(f.args(&wrapped).unwrap(), vec!["retrieve", "invoice"]);
        let wrong = format!("(cd /different && {invocation})");
        assert!(f.args(&wrong).is_none());
    }
    #[test]
    fn canonical_native_save_call_is_recovered_without_executing_it() {
        let f = Fixture::new();
        let request = "a".repeat(64);
        let token = "b".repeat(32);
        let operation =
            format!("native-result --request {request} --token {token} --input - --json");
        let command = format!(
            "{} <<'NATIVE_RESULT'\n{{\"status\":\"no-learning\"}}\nNATIVE_RESULT",
            f.command(&f.installed(), &operation)
        );
        let (actual_request, actual_token, payload) =
            save_call(&command, &f.root, &f.runtime).unwrap();
        assert_eq!(actual_request, request);
        assert_eq!(actual_token, token);
        assert_eq!(payload["status"], "no-learning");
    }
    #[test]
    fn native_evidence_rejects_other_roots_composition_and_substitution() {
        let f = Fixture::new();
        let base = f.command(&f.runtime, "retrieve invoice");
        assert!(f.args(&format!("{base}; echo forged")).is_none());
        assert!(f.args(&format!("{base} && echo forged")).is_none());
        assert!(
            f.args(&f.command(&f.runtime, "retrieve \"$(echo forged)\""))
                .is_none()
        );
        assert!(
            f.args(&format!(
                "{} memory --root /different retrieve invoice",
                shell_words::quote(&f.runtime.to_string_lossy())
            ))
            .is_none()
        );
        assert!(f.args("infra-engine memory retrieve invoice").is_none());
    }
    #[test]
    fn historical_evidence_aliases_keep_their_original_argument_shape() {
        let f = Fixture::new();
        for alias in ["infra-cli", "infra-runtime"] {
            let command = format!(
                "{alias} --root {} retrieve invoice",
                shell_words::quote(&f.root.to_string_lossy())
            );
            assert_eq!(f.args(&command).unwrap(), vec!["retrieve", "invoice"]);
        }
    }
}
