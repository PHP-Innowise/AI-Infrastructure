use crate::{
    Result, artifact, isolation,
    process::{self},
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
pub fn oracle_digest(root: &Path) -> Result<String> {
    let root = artifact::ordinary(root)?;
    if root.is_file() {
        if artifact::mode(&root)? & 0o7000 != 0 {
            return Err(artifact::invalid("special oracle permissions refused"));
        }
        let (raw, mode) = artifact::read_file(&root, 512 * 1024 * 1024)?;
        return artifact::digest(&json!(["file", artifact::sha(&raw), mode]));
    }
    let mut entries = vec![json!([".", "directory", artifact::mode(&root)?])];
    fn dirs(root: &Path, dir: &Path, entries: &mut Vec<Value>) -> Result<()> {
        for e in std::fs::read_dir(dir)? {
            let e = e?;
            let p = e.path();
            let m = std::fs::symlink_metadata(&p)?;
            if infra_core::fs::linked(&m) {
                return Err(artifact::invalid("oracle link refused"));
            }
            if m.is_dir() {
                let name = artifact::snapshot_name(
                    p.strip_prefix(root)
                        .map_err(|_| artifact::invalid("oracle root"))?,
                )?;
                entries.push(json!([name, "directory", artifact::mode(&p)?]));
                dirs(root, &p, entries)?;
            }
            if entries.len() > 100000 {
                return Err(artifact::invalid("oracle artifact count exceeded"));
            }
        }
        Ok(())
    }
    dirs(&root, &root, &mut entries)?;
    for f in artifact::image(&root, artifact::ImageKind::Oracle, None)? {
        if f.mode & 0o7000 != 0 {
            return Err(artifact::invalid("special oracle permissions refused"));
        }
        entries.push(json!([f.path, "file", artifact::sha(&f.bytes), f.mode]));
    }
    entries.sort_by(|a, b| a[0].as_str().cmp(&b[0].as_str()));
    artifact::digest(&json!(entries))
}
pub fn verifier_digest(case: &Value) -> Result<String> {
    if !case.is_object() {
        return Err(artifact::invalid("invalid verifier artifacts"));
    }
    let mut images = vec![];
    for field in ["read_roots", "argv"] {
        if case.get(field).is_some()
            && !case[field]
                .as_array()
                .is_some_and(|rows| rows.iter().all(Value::is_string))
        {
            return Err(artifact::invalid("invalid verifier artifacts"));
        }
    }
    for raw in case["read_roots"].as_array().into_iter().flatten() {
        let p = artifact::ordinary(Path::new(raw.as_str().unwrap()))?;
        images.push(json!([
            infra_core::fs::canonicalize(&p)?,
            oracle_digest(&p)?
        ]));
    }
    for (index, raw) in case["argv"].as_array().into_iter().flatten().enumerate() {
        let p = Path::new(raw.as_str().unwrap());
        if p.is_absolute() && p.is_file() {
            let p = if index == 0 {
                infra_core::fs::canonicalize(p)?
            } else {
                artifact::ordinary(p)?
            };
            images.push(json!([
                infra_core::fs::canonicalize(&p)?,
                oracle_digest(&p)?
            ]));
        }
    }
    artifact::digest(&json!([case, images]))
}
pub fn verify(snapshot: &Path, answer: &Path, case: &Value) -> Value {
    let started = Instant::now();
    let mut result = json!({"status":"unavailable","correct":null,"verifier_digest":null,"artifact_digest":null,"seconds":null});
    let evaluation = (|| -> Result<bool> {
        let verifier = verifier_digest(case)?;
        result["verifier_digest"] = verifier.clone().into();
        let image = artifact::candidate_digest(snapshot)?;
        let answer_bytes = artifact::read_file(answer, 1024 * 1024)?.0;
        result["artifact_digest"] =
            artifact::digest(&json!([image, artifact::sha(&answer_bytes)]))?.into();
        let correct = match case["mode"].as_str() {
            Some("answer") => {
                if !artifact::exact(case, &["mode", "equals"]) || !case["equals"].is_string() {
                    return Err(artifact::invalid("invalid answer oracle"));
                }
                std::str::from_utf8(&answer_bytes)
                    .map_err(|_| artifact::invalid("answer encoding"))?
                    .trim()
                    == case["equals"].as_str().unwrap().trim()
            }
            Some("patch") => {
                if !artifact::exact(case, &["mode", "argv", "timeout", "read_roots"]) {
                    return Err(artifact::invalid("invalid patch oracle"));
                }
                let argv = case["argv"]
                    .as_array()
                    .ok_or_else(|| artifact::invalid("invalid oracle argv"))?
                    .iter()
                    .map(|s| {
                        s.as_str()
                            .filter(|s| !s.is_empty())
                            .map(String::from)
                            .ok_or_else(|| artifact::invalid("invalid oracle argv"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let timeout = case["timeout"]
                    .as_f64()
                    .filter(|t| t.is_finite() && *t > 0.0)
                    .ok_or_else(|| artifact::invalid("invalid oracle timeout"))?;
                let roots = case["read_roots"]
                    .as_array()
                    .ok_or_else(|| artifact::invalid("invalid oracle roots"))?
                    .iter()
                    .map(|s| {
                        s.as_str()
                            .map(PathBuf::from)
                            .ok_or_else(|| artifact::invalid("invalid oracle roots"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let program = argv
                    .first()
                    .map(PathBuf::from)
                    .ok_or_else(|| artifact::invalid("missing native oracle"))?;
                let p = artifact::ordinary(&program)?;
                if !p.is_absolute()
                    || !p.is_file()
                    || artifact::overlaps(&p, snapshot)?
                    || !roots.iter().any(|r| p.starts_with(r))
                {
                    return Err(artifact::invalid("external sealed native oracle required"));
                }
                let temporary = artifact::Scratch::new(
                    snapshot
                        .parent()
                        .ok_or_else(|| artifact::invalid("candidate parent"))?,
                    "outcome-verify",
                )?;
                let candidate = temporary.0.join("candidate");
                artifact::copy_native_tree(
                    snapshot,
                    &candidate,
                    artifact::ImageKind::Candidate,
                    None,
                )?;
                if artifact::candidate_digest(&candidate)? != image {
                    return Err(artifact::invalid("candidate copy changed"));
                }
                let spec = isolation::verifier(&candidate, &argv, &roots)?;
                let (code, raw) = process::capture(&spec, Duration::from_secs_f64(timeout), 65536)?;
                if code != 0 {
                    return Err(artifact::invalid("isolated oracle unavailable"));
                }
                let receipt = artifact::strict(&raw)?;
                if !artifact::exact(&receipt, &["schema", "completed", "correct"])
                    || receipt["schema"] != "task-outcome-verifier-v1"
                    || receipt["completed"] != true
                    || !receipt["correct"].is_boolean()
                {
                    return Err(artifact::invalid("invalid oracle receipt"));
                }
                receipt["correct"] == true
            }
            _ => return Err(artifact::invalid("invalid oracle mode")),
        };
        if verifier_digest(case)? != verifier {
            return Err(artifact::invalid("oracle artifacts changed"));
        }
        Ok(correct)
    })();
    if let Ok(correct) = evaluation {
        result["correct"] = correct.into();
        result["status"] = if correct { "passed" } else { "failed" }.into();
    }
    result["seconds"] = json!((started.elapsed().as_secs_f64() * 10000.0).round() / 10000.0);
    result
}
