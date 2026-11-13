use crate::{Result, artifact, isolation, oracle, process, response};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::{Duration, Instant},
};
pub const MOUNT_NAME: &str = ".memory-citation-corpus";
pub const MAX_PROMPT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PACKET_BYTES: usize = 32 * 1024 * 1024;
pub fn configuration(task: &Value, case: &Value) -> Result<Value> {
    let p = response::validate(&task["response_contract"])?;
    if p["schema"] != "memory-response-contract-v2"
        || p["case_id"] != task["id"]
        || !artifact::exact(&case["response"], &["reference", "answer_field"])
    {
        return Err(artifact::invalid(
            "v2 requires exact public case and private reference",
        ));
    }
    let field = &case["response"]["answer_field"];
    if task["task_mode"] == "answer" {
        let f = field
            .as_str()
            .ok_or_else(|| artifact::invalid("answer intake requires explicit field"))?;
        let mut fields = serde_json::Map::new();
        fields.insert(f.into(), json!({"type":"string","nullable":false}));
        if p["answer_fields"] != Value::Object(fields) || !case["equals"].is_string() {
            return Err(artifact::invalid(
                "answer intake grades exactly one nonnullable string field",
            ));
        }
    } else if !field.is_null() {
        return Err(artifact::invalid("patch intake requires null answer_field"));
    }
    let reference = case["response"]["reference"]
        .as_str()
        .ok_or_else(|| artifact::invalid("invalid private reference"))?;
    if response::check(&p, reference)?["valid"] != true {
        return Err(artifact::invalid(
            "private reference violates public contract",
        ));
    }
    if let Some(f) = field.as_str() {
        let v = artifact::strict(
            response::body(reference)
                .ok_or_else(|| artifact::invalid("reference body"))?
                .as_bytes(),
        )?;
        if v["answer"][f].as_str().unwrap_or("").trim()
            != case["equals"].as_str().unwrap_or("").trim()
        {
            return Err(artifact::invalid("private reference disagrees with oracle"));
        }
    }
    Ok(p)
}
pub fn prompt(task: &Value, case: &Value) -> Result<Vec<u8>> {
    let p = configuration(task, case)?;
    let base = task["prompt"]
        .as_str()
        .filter(|s| !s.contains('\0'))
        .ok_or_else(|| artifact::invalid("invalid v2 prompt"))?;
    let mut instructions = format!(
        "\n\nCitation evidence is the immutable pre-task snapshot under {MOUNT_NAME}/. Read the listed path beneath that directory; cite its original relative path, without the snapshot prefix. Edits to working files do not change these citation bytes. Unlisted runtime files remain ineligible. The complete corpus is common to both arms and is additional public context; it is not an absence-of-memory control.\n"
    );
    if let Some(field) = case["response"]["answer_field"].as_str() {
        instructions.push_str(&format!(
            "The task answer graded by the private answer oracle is the string field {}.\n",
            serde_json::to_string(field)?
        ));
    } else {
        instructions.push_str(
            "Patch correctness is graded independently of response shape and literal citations.\n",
        );
    }
    let raw = format!("{base}{instructions}{}", response::render(&p)?).into_bytes();
    if raw.len() > MAX_PROMPT_BYTES {
        return Err(artifact::invalid(
            "complete effective prompt exceeds byte bound",
        ));
    }
    Ok(raw)
}
pub fn sources(
    root: &Path,
    public: &Value,
    exact: bool,
) -> Result<(BTreeMap<String, Vec<u8>>, String)> {
    let root = artifact::ordinary(root)?;
    if !root.is_dir() {
        return Err(artifact::invalid("ordinary citation snapshot required"));
    }
    let public = response::validate(public)?;
    let mut sources = BTreeMap::new();
    let mut rows = vec![];
    let mut dirs = BTreeSet::from([".".to_string()]);
    for e in public["citation_corpus"]["files"]
        .as_array()
        .ok_or_else(|| artifact::invalid("citation files required"))?
    {
        let name = e["path"].as_str().unwrap();
        let path = artifact::relative(name)?;
        let absolute = root.join(&path);
        if artifact::mode(&absolute)? & 0o7000 != 0 {
            return Err(artifact::invalid("special citation permissions refused"));
        }
        let (raw, mode) = artifact::read_file(&absolute, 1048576)?;
        artifact::public_text(
            std::str::from_utf8(&raw).map_err(|_| artifact::invalid("invalid source UTF-8"))?,
        )?;
        rows.push(json!([name, artifact::sha(&raw), raw.len(), mode]));
        sources.insert(name.into(), raw);
        for parent in path.ancestors().skip(1) {
            dirs.insert(if parent.as_os_str().is_empty() {
                ".".into()
            } else {
                parent.to_string_lossy().replace('\\', "/")
            });
        }
    }
    response::citations(&public, "", &sources)?;
    if exact {
        let mut found_dirs = BTreeSet::from([".".to_string()]);
        let mut found_files = BTreeSet::new();
        fn walk(
            root: &Path,
            at: &Path,
            dirs: &mut BTreeSet<String>,
            files: &mut BTreeSet<String>,
        ) -> Result<()> {
            for entry in std::fs::read_dir(at)? {
                let p = entry?.path();
                let name = p
                    .strip_prefix(root)
                    .map_err(|_| artifact::invalid("citation root"))?
                    .to_string_lossy()
                    .replace('\\', "/");
                let m = std::fs::symlink_metadata(&p)?;
                if infra_core::fs::linked(&m) || artifact::mode(&p)? & 0o7000 != 0 {
                    return Err(artifact::invalid("citation link or special permission"));
                }
                if m.is_dir() {
                    dirs.insert(name);
                    walk(root, &p, dirs, files)?;
                } else if m.is_file() {
                    files.insert(name);
                } else {
                    return Err(artifact::invalid("citation special file"));
                }
                if dirs.len() + files.len() > 8192 {
                    return Err(artifact::invalid("citation entry bound"));
                }
            }
            Ok(())
        }
        walk(&root, &root, &mut found_dirs, &mut found_files)?;
        if found_dirs != dirs || found_files != sources.keys().cloned().collect() {
            return Err(artifact::invalid("citation snapshot membership changed"));
        }
        for name in found_dirs {
            rows.push(json!([
                name,
                "directory",
                artifact::mode(&root.join(&name))?
            ]));
        }
    }
    Ok((sources, artifact::digest(&json!(rows))?))
}
pub fn prepare(task: &Value, case: &Value, root: &Path, target: &Path) -> Result<Value> {
    let public = configuration(task, case)?;
    let (s, _) = sources(root, &public, false)?;
    if response::citations(&public, case["response"]["reference"].as_str().unwrap(), &s)?["valid"]
        != true
    {
        return Err(artifact::invalid(
            "private reference has invalid literal citations",
        ));
    }
    if target.exists() {
        return Err(artifact::invalid("fresh citation snapshot required"));
    }
    artifact::private_dir(target)?;
    for (name, raw) in s {
        artifact::fresh_file(&target.join(name), &raw, 0o444)?;
    }
    let (_, image) = sources(target, &public, true)?;
    let prompt = prompt(task, case)?;
    Ok(
        json!({"schema":"memory-response-intake-v1","path":target,"source_image_sha256":image,"contract_sha256":response::contract_digest(&public)?,"prompt_sha256":artifact::sha(&prompt),"prompt_bytes":prompt.len()}),
    )
}
pub type PreparedResponseInputs = (BTreeMap<String, Vec<u8>>, Vec<u8>);

pub fn validate(
    task: &Value,
    case: &Value,
    prepared: &Value,
    root: Option<&Path>,
) -> Result<PreparedResponseInputs> {
    let public = configuration(task, case)?;
    let p = prompt(task, case)?;
    let declared = Path::new(
        prepared["path"]
            .as_str()
            .ok_or_else(|| artifact::invalid("missing prepared root"))?,
    );
    let (s, image) = sources(root.unwrap_or(declared), &public, true)?;
    if prepared["schema"] != "memory-response-intake-v1"
        || prepared["source_image_sha256"] != image
        || prepared["contract_sha256"] != response::contract_digest(&public)?
        || prepared["prompt_sha256"] != artifact::sha(&p)
        || prepared["prompt_bytes"] != p.len()
    {
        return Err(artifact::invalid(
            "sealed response corpus, contract or prompt changed",
        ));
    }
    Ok((s, p))
}
pub fn evidence(packet: &Value) -> Result<Value> {
    let public = response::validate(&packet["contract"])?;
    let encoded = packet["sources"]
        .as_object()
        .ok_or_else(|| artifact::invalid("source map required"))?;
    let mut s = BTreeMap::new();
    for (name, raw) in encoded {
        s.insert(
            name.clone(),
            STANDARD
                .decode(
                    raw.as_str()
                        .ok_or_else(|| artifact::invalid("source base64 required"))?,
                )
                .map_err(|_| artifact::invalid("invalid source base64"))?,
        );
    }
    response::citations(&public, "", &s)?;
    let capture = &packet["capture"];
    if !artifact::exact(capture, &["schema", "messages", "overflow"])
        || capture["schema"] != "memory-assistant-capture-v1"
        || !capture["overflow"].is_boolean()
    {
        return Err(artifact::invalid("invalid assistant capture"));
    }
    let a = response::make_artifact(&public, &capture["messages"], &packet["context"])?;
    let mut c = response::citations(&public, a["selection"]["text"].as_str().unwrap_or(""), &s)?;
    if capture["overflow"] == true {
        c = json!({"valid":false,"errors":["assistant transcript exceeded declared limits"]});
    }
    let answer = if c["valid"] == true {
        let text = a["selection"]["text"].as_str().unwrap();
        if let Some(field) = packet["answer_field"].as_str() {
            artifact::strict(response::body(text).unwrap().as_bytes())?["answer"][field].clone()
        } else {
            text.into()
        }
    } else {
        Value::Null
    };
    Ok(
        json!({"schema":"memory-response-evidence-v1","artifact":a,"citation_check":c,"answer_text":answer}),
    )
}
pub fn worker(args: &[String]) -> Result<Option<i32>> {
    if args.first().map(String::as_str) != Some("__response-oracle") {
        return Ok(None);
    }
    if args.len() != 2 {
        return Ok(Some(2));
    }
    let p = artifact::read_json(Path::new(&args[1]), MAX_PACKET_BYTES)?;
    let _clock = crate::clock::request(&p)?;
    let value = evidence(&p)?;
    print!(
        "{}",
        infra_core::domain::json::serialize(&value, false, false, false, false)?
    );
    Ok(Some(0))
}
#[expect(
    clippy::too_many_arguments,
    reason = "Sealed response grading binds the executable, snapshot, capture, task, verifier context, corpus and optional capture digest."
)]
pub fn grade(
    executable: &Path,
    snapshot: &Path,
    capture: &Path,
    case: &Value,
    task: &Value,
    prepared: &Value,
    citation_root: &Path,
    context: &Value,
    capture_digest: Option<&str>,
) -> Value {
    let started = Instant::now();
    let mut out = json!({"status":"unavailable","correct":null,"verifier_digest":null,"artifact_digest":null,"seconds":null});
    let graded = (|| -> Result<()> {
        let verifier = oracle::verifier_digest(case)?;
        out["verifier_digest"] = verifier.clone().into();
        if context["verifier_sha256"] != verifier
            || context["source_image_sha256"] != prepared["source_image_sha256"]
        {
            return Err(artifact::invalid("response association changed"));
        }
        let (s, _) = validate(task, case, prepared, Some(citation_root))?;
        let captured = artifact::read_json(capture, 8 * 1024 * 1024)?;
        if capture_digest.is_some_and(|h| artifact::digest(&captured).ok().as_deref() != Some(h)) {
            return Err(artifact::invalid("capture association changed"));
        }
        let sources = s
            .iter()
            .map(|(name, raw)| (name.clone(), Value::String(STANDARD.encode(raw))))
            .collect::<serde_json::Map<_, _>>();
        let packet = json!({"contract":task["response_contract"],"capture":captured,"context":context,"answer_field":case["response"]["answer_field"],"sources":sources});
        let raw = serde_json::to_vec(&packet)?;
        if raw.len() > MAX_PACKET_BYTES {
            return Err(artifact::invalid("response packet byte bound"));
        }
        let temporary = artifact::Scratch::new(
            snapshot
                .parent()
                .ok_or_else(|| artifact::invalid("snapshot parent"))?,
            "response-oracle",
        )?;
        let private = temporary.0.join("private");
        let candidate = temporary.0.join("candidate");
        artifact::private_dir(&private)?;
        artifact::private_dir(&candidate)?;
        let request = private.join("request.json");
        artifact::fresh_file(&request, &raw, 0o600)?;
        let argv = vec![
            infra_core::fs::canonicalize(executable)?
                .to_string_lossy()
                .into(),
            "__response-oracle".into(),
            request.to_string_lossy().into(),
        ];
        let spec = isolation::verifier(&candidate, &argv, std::slice::from_ref(&private))?;
        let (code, raw) = process::capture(&spec, Duration::from_secs(30), 8 * 1024 * 1024)?;
        if code != 0 {
            return Err(artifact::invalid("isolated response evidence unavailable"));
        }
        let receipt = artifact::strict(&raw)?;
        if receipt != evidence(&packet)? {
            return Err(artifact::invalid("response receipt association mismatch"));
        }
        out["response"] = receipt.clone();
        out["artifact_digest"] = artifact::digest(&receipt["artifact"])?.into();
        validate(task, case, prepared, Some(citation_root))?;
        if receipt["citation_check"]["valid"] != true {
            out["status"] = "failed".into();
            out["correct"] = false.into();
        } else {
            let answer = private.join("answer.txt");
            artifact::fresh_file(
                &answer,
                receipt["answer_text"]
                    .as_str()
                    .ok_or_else(|| artifact::invalid("answer evidence missing"))?
                    .as_bytes(),
                0o600,
            )?;
            let mut semantic = case.clone();
            semantic.as_object_mut().unwrap().remove("response");
            let semantic = oracle::verify(snapshot, &answer, &semantic);
            out["status"] = semantic["status"].clone();
            out["correct"] = semantic["correct"].clone();
            out["artifact_digest"] = artifact::digest(&json!([
                out["artifact_digest"],
                semantic["artifact_digest"]
            ]))?
            .into();
            out["semantic"] = semantic;
        }
        validate(task, case, prepared, Some(citation_root))?;
        if oracle::verifier_digest(case)? != verifier {
            return Err(artifact::invalid("verifier changed"));
        }
        Ok(())
    })();
    if graded.is_err() {
        out["status"] = "unavailable".into();
        out["correct"] = Value::Null;
    }
    out["seconds"] = json!((started.elapsed().as_secs_f64() * 10000.0).round() / 10000.0);
    out
}
