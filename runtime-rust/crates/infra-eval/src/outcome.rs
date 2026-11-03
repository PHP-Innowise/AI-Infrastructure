use crate::executable;
use crate::{
    Result, artifact, delivery, intake, isolation, oracle,
    process::{self, Budget, CommandSpec},
    reconstruction::{self},
    response, rng, transcript, treatments,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
pub const ARMS: [&str; 8] = [
    "off",
    "capsule",
    "navigation",
    "recall",
    "ticket",
    "selective",
    "consolidate",
    "consolidate-verify",
];
pub fn arm_settings(arm: &str) -> Result<Value> {
    Ok(match arm {
        "off" => json!({}),
        "capsule" => json!({"capsule_profile":"full"}),
        "navigation" => json!({"capsule_profile":"navigation"}),
        "recall" => json!({"capsule_profile":"recall"}),
        "ticket" => json!({"capsule_profile":"recall","ticket_files":true}),
        "selective" => json!({"capsule_profile":"recall"}),
        "consolidate" => {
            json!({"capsule_profile":"recall","automatic_consolidation":true,"automatic_consolidation_verification":false})
        }
        "consolidate-verify" => {
            json!({"capsule_profile":"recall","automatic_consolidation":true,"automatic_consolidation_verification":true})
        }
        _ => return Err(artifact::invalid("unknown arm")),
    })
}
pub fn configure(root: &Path, arm: &str) -> Result<Value> {
    let p = root.join("project-brain/config/runtime.json");
    let mut c = if p.is_file() {
        artifact::read_json(&p, artifact::MAX_FILE)?
    } else {
        json!({})
    };
    let settings = arm_settings(arm)?;
    for (k, v) in settings.as_object().unwrap() {
        c[k] = v.clone();
    }
    artifact::write_json(&p, &c)?;
    Ok(c)
}
pub fn config_digest(root: &Path, arm: &str) -> Result<String> {
    let p = root.join("project-brain/config/runtime.json");
    let mut c = if p.is_file() {
        artifact::read_json(&p, artifact::MAX_FILE)?
    } else {
        json!({})
    };
    for (k, v) in arm_settings(arm)?.as_object().unwrap() {
        c[k] = v.clone();
    }
    artifact::digest(&c)
}
pub fn toml(v: &Value) -> Result<String> {
    match v {
        Value::Object(m) => Ok(format!(
            "{{ {} }}",
            m.iter()
                .map(|(k, v)| Ok(format!("{} = {}", serde_json::to_string(k)?, toml(v)?)))
                .collect::<Result<Vec<_>>>()?
                .join(", ")
        )),
        Value::Array(a) => Ok(format!(
            "[{}]",
            a.iter().map(toml).collect::<Result<Vec<_>>>()?.join(", ")
        )),
        Value::Null => Err(artifact::invalid("null cannot be encoded in native TOML")),
        _ => Ok(serde_json::to_string(v)?),
    }
}
pub fn hook_arguments(root: &Path, selective: bool, runtime: &Path) -> Result<Vec<String>> {
    let h = artifact::read_json(&root.join(".codex/hooks.json"), 1048576)?;
    let mut groups = vec![];
    for g in h["hooks"]["UserPromptSubmit"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let mut hooks = vec![];
        for handler in g["hooks"].as_array().into_iter().flatten() {
            if handler["command"].as_str().is_some_and(|s| {
                artifact::strip_trailing_space(s).ends_with("working-memory-read.sh")
                    || s.contains("working-memory-read")
                    || s.contains("hook-prompt")
            }) {
                let mut handler = handler.clone();
                if selective {
                    let folder = root.join("memory-bank/local/experiments");
                    artifact::private_dir(&folder)?;
                    let rules = folder.join("rules.json");
                    artifact::write_json(&rules, &treatments::rules())?;
                    let words = vec![
                        runtime.to_string_lossy().into_owned(),
                        "memory-selective-retrieve-pilot".into(),
                        "--enabled".into(),
                        "--rules".into(),
                        rules.to_string_lossy().into(),
                        "--audit".into(),
                        folder.join("route.jsonl").to_string_lossy().into(),
                        "--hook".into(),
                        handler["command"].as_str().unwrap().into(),
                    ];
                    handler["command"] = shell_words::join(words).into();
                }
                hooks.push(handler);
            }
        }
        if !hooks.is_empty() {
            let mut g = g.clone();
            g["hooks"] = json!(hooks);
            groups.push(g);
        }
    }
    if groups.is_empty() {
        return Err(artifact::invalid("no memory prompt hook"));
    }
    Ok(vec![
        "--dangerously-bypass-hook-trust".into(),
        "-c".into(),
        "features.hooks=true".into(),
        "-c".into(),
        format!("hooks.UserPromptSubmit={}", toml(&json!(groups))?),
    ])
}
pub fn codex_command(
    codex: &str,
    root: &Path,
    arm: &str,
    prompt: &str,
    model: Option<&str>,
    runtime: &Path,
) -> Result<Vec<String>> {
    let mut command = vec![
        codex.into(),
        "exec".into(),
        "--json".into(),
        "--ephemeral".into(),
        "--ignore-user-config".into(),
        "--skip-git-repo-check".into(),
        "-s".into(),
        "workspace-write".into(),
        "-C".into(),
        root.to_string_lossy().into(),
    ];
    if let Some(model) = model {
        command.extend(["-m".into(), model.into()]);
    }
    if arm == "off" {
        command.extend(["-c".into(), "features.hooks=false".into()]);
    } else {
        command.extend(hook_arguments(root, arm == "selective", runtime)?);
    }
    command.push(prompt.into());
    Ok(command)
}
pub fn event_paths(item: &Value, root: &Path) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    if item["type"] == "file_change" {
        for c in item["changes"].as_array().into_iter().flatten() {
            if let Some(p) = c["path"].as_str()
                && let Some(p) = project_path(p, root, root)
            {
                found.insert(p);
            }
        }
    } else if item["type"] == "command_execution"
        && let Some(command) = item["command"].as_str()
    {
        let outer = shell_words::split(command).unwrap_or_default();
        let script = if outer.len() >= 3
            && Path::new(&outer[0])
                .file_name()
                .is_some_and(|n| ["bash", "sh", "zsh", "dash"].iter().any(|s| n == *s))
            && ["-c", "-lc", "-ic", "-lic"].contains(&outer[1].as_str())
        {
            outer[2].as_str()
        } else {
            command
        };
        for (word, _changed, base) in infra_core::continuity::shell_file_words(script, root, "sh") {
            if let Some(p) = project_path(&word, &base, root) {
                found.insert(p);
            }
        }
    }
    found
}
pub fn project_path(word: &str, base: &Path, root: &Path) -> Option<String> {
    let word = word.trim_matches(|c: char| c.is_whitespace() || "'\"`,;()[]{}<>".contains(c));
    if word.is_empty() || word.starts_with('-') || word.len() > 400 || word.contains('\0') {
        return None;
    }
    let word = regex::Regex::new(r"(?::\d+(?::\d+)?|#L\d+(?:-L?\d+)?)$")
        .ok()?
        .replace(word, "");
    let p = Path::new(word.as_ref());
    let p = if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    };
    let mut normalized = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::CurDir => {}
            _ => normalized.push(c.as_os_str()),
        }
    }
    let p = normalized.strip_prefix(root).ok()?;
    if p.as_os_str().is_empty() {
        None
    } else {
        Some(p.to_string_lossy().replace('\\', "/"))
    }
}
fn manifests(root: &Path) -> Result<BTreeSet<PathBuf>> {
    let mut out = BTreeSet::new();
    for dir in [
        "memory-bank/local/retrieval-manifests",
        "project-brain/control/retrieval-manifests",
    ] {
        if root.join(dir).is_dir() {
            for e in std::fs::read_dir(root.join(dir))? {
                let p = e?.path();
                if p.extension().is_some_and(|e| e == "json") {
                    if out.len() >= 1000 {
                        return Err(artifact::invalid("manifest scan bound"));
                    }
                    out.insert(p);
                }
            }
        }
    }
    Ok(out)
}
fn health(root: &Path) -> Result<Vec<Value>> {
    let p = root.join("memory-bank/local/refresh-health.ndjson");
    if !p.exists() {
        Ok(vec![])
    } else {
        let (raw, _) = artifact::read_file(&p, artifact::MAX_FILE)?;
        raw.split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .map(artifact::strict)
            .collect()
    }
}
pub struct ArmRequest<'a> {
    pub command: Vec<String>,
    pub corpus: &'a Path,
    pub gold: &'a [String],
    pub cap: usize,
    pub timeout: Duration,
    pub endpoint: &'a str,
    pub env: BTreeMap<String, String>,
    pub prompt_input: Vec<u8>,
    pub response_contract: Option<&'a Value>,
    pub answer: Option<&'a Path>,
    pub raw_events: Option<&'a Path>,
    pub saved: &'a [String],
}
pub fn run_arm(r: ArmRequest<'_>) -> Result<Value> {
    if !["first-touch", "task-complete"].contains(&r.endpoint) {
        return Err(artifact::invalid("unknown endpoint"));
    }
    let strict = r.endpoint == "task-complete";
    let started = Instant::now();
    let before = manifests(r.corpus)?;
    let initial = health(r.corpus)?;
    let known = delivery::known_paths(r.corpus)?;
    let mut first = None;
    let mut numbers = BTreeMap::<String, usize>::new();
    let spec = CommandSpec {
        argv: r.command,
        cwd: r.corpus.into(),
        env: r.env,
        input: r.prompt_input.clone(),
    };
    let mut budget = Budget::native(r.timeout, r.cap);
    budget.strict_events = strict;
    budget.error_diagnostics_terminal = false;
    let session = process::execute(&spec, &budget, r.raw_events, None, |event, session| {
        if event["type"] == "turn.started" && strict {
            numbers.clear();
        }
        let item = &event["item"];
        if !["item.started", "item.completed"].contains(&event["type"].as_str().unwrap_or("")) {
            return None;
        }
        let id = item["id"].as_str().unwrap_or("");
        let n = *numbers.entry(id.into()).or_insert(session.calls);
        if first.is_none()
            && event_paths(item, r.corpus)
                .iter()
                .any(|p| r.gold.contains(p))
        {
            first = Some(n);
            if !strict {
                return Some("gold".into());
            }
        }
        None
    })?;
    let after = manifests(r.corpus)?;
    let final_health = health(r.corpus)?;
    let telemetry = final_health.len() >= initial.len();
    let added = final_health.iter().skip(initial.len()).collect::<Vec<_>>();
    let mut points = BTreeMap::<String, usize>::new();
    let mut capsule_tokens = 0u64;
    let mut saved = false;
    let mut manifests_complete = true;
    for path in after.difference(&before) {
        match artifact::read_json(path, 1048576) {
            Ok(m) => {
                let point = m["entry_point"].as_str().unwrap_or("unreadable");
                *points.entry(point.into()).or_default() += 1;
                if ["refresh", "hook-context"].contains(&point) {
                    capsule_tokens += m["token_estimates"]["total"].as_u64().unwrap_or(0);
                    saved |= m["selected"].as_array().into_iter().flatten().any(|e| {
                        e["path"]
                            .as_str()
                            .is_some_and(|p| r.saved.iter().any(|s| s == p))
                    });
                }
            }
            Err(_) => manifests_complete = false,
        }
    }
    let capsule_bytes = if added
        .iter()
        .any(|r| r["capsule"]["bytes"].as_u64().is_none())
    {
        None
    } else {
        Some(
            added
                .iter()
                .filter_map(|r| r["capsule"]["bytes"].as_u64())
                .sum::<u64>(),
        )
    };
    let delivered = points.get("refresh").copied().unwrap_or(0)
        + points.get("hook-context").copied().unwrap_or(0)
        > 0;
    let answered = session.completed
        && first.is_none()
        && r.gold
            .iter()
            .any(|p| transcript::mentioned(p, &session.answer));
    let mut out = session.value();
    out["calls_to_gold"] = json!(first);
    out["calls_to_answer"] = if answered {
        json!(session.calls)
    } else {
        Value::Null
    };
    out["saved_delivered"] = saved.into();
    out["capsule_delivered"] = delivered.into();
    out["hook_ran"] = (final_health.len() > initial.len()).into();
    out["ticket_files_delivered"] = added
        .iter()
        .any(|r| r["capsule"]["ticket_files"].as_u64().unwrap_or(0) > 0)
        .into();
    out["capsule_tokens"] = capsule_tokens.into();
    out["capsule_bytes"] = json!(capsule_bytes);
    out["manifests"] = json!(points);
    if strict {
        out["telemetry_complete"] = (telemetry && manifests_complete).into();
        out["delivery_proof"] = delivery::collect(
            r.corpus,
            &after.difference(&before).cloned().collect::<Vec<_>>(),
            &known,
            Some(started + r.timeout),
        );
    }
    if let Some(public) = r.response_contract {
        response::validate(public)?;
        let l = &public["limits"];
        let mut messages = vec![];
        let mut bytes = 0usize;
        let mut overflow = session.message_overflow;
        for message in &session.messages {
            if messages.len() >= l["max_messages"].as_u64().unwrap_or(0) as usize
                || message.len() > l["max_message_bytes"].as_u64().unwrap_or(0) as usize
                || bytes + message.len() > l["max_total_bytes"].as_u64().unwrap_or(0) as usize
            {
                overflow = true;
            } else if !overflow {
                bytes += message.len();
                messages.push(message.clone());
            }
        }
        let capture =
            json!({"schema":"memory-assistant-capture-v1","messages":messages,"overflow":overflow});
        out["response_capture_sha256"] = artifact::digest(&capture)?.into();
        out["response_capture"] = capture;
        if let Some(answer) = r.answer {
            artifact::write_json(answer, &out["response_capture"])?;
        }
    } else if let Some(answer) = r.answer {
        artifact::fresh_file(answer, session.answer.as_bytes(), 0o600)?;
    }
    if !r.prompt_input.is_empty() {
        out["prompt_transport"] = json!({"method":"stdin","bytes":r.prompt_input.len(),"sha256":artifact::sha(&r.prompt_input),"model_receipt":"unknown"});
    }
    out.as_object_mut().unwrap().remove("answer");
    Ok(out)
}
pub fn valid_arm(arm: &str, runs: &[Value]) -> bool {
    if runs.is_empty() || runs.iter().any(|r| r["telemetry_complete"] == false) {
        return false;
    }
    match arm {
        "capsule" => runs.iter().all(|r| r["capsule_delivered"] == true),
        "navigation" => runs
            .iter()
            .all(|r| r["hook_ran"] == true && r["capsule_delivered"] != true),
        "selective" => runs
            .iter()
            .all(|r| r["hook_ran"] == true && r["routing"]["status"] == "observed"),
        "recall" | "ticket" | "consolidate" | "consolidate-verify" => {
            runs.iter().all(|r| r["hook_ran"] == true)
        }
        "off" => runs
            .iter()
            .all(|r| r["hook_ran"] != true && r["capsule_delivered"] != true),
        _ => false,
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub arms: Vec<String>,
    pub repeat: usize,
    pub cap: usize,
    pub timeout: f64,
    pub model: String,
    pub codex_version: String,
    pub endpoint: String,
    pub retry_policy: String,
    pub git_history: String,
}
impl Settings {
    pub fn new(
        arms: Vec<String>,
        repeat: usize,
        cap: usize,
        timeout: f64,
        model: String,
        codex: &str,
    ) -> Result<Self> {
        if arms.len() != 2
            || arms[0] == arms[1]
            || arms
                .iter()
                .any(|a| !ARMS.contains(&a.as_str()) || a == "ticket")
            || repeat == 0
            || cap == 0
            || !timeout.is_finite()
            || timeout <= 0.0
            || model.trim().is_empty()
        {
            return Err(artifact::invalid(
                "task-complete requires two arms, model, positive budgets",
            ));
        }
        let spec = CommandSpec {
            argv: vec![codex.into(), "--version".into()],
            cwd: std::env::current_dir()?,
            env: reconstruction::clean_env(),
            input: vec![],
        };
        let (code, raw) = process::capture(&spec, Duration::from_secs(10), 16384)?;
        if code != 0 {
            return Err(artifact::invalid("native CLI version unavailable"));
        }
        Ok(Self {
            arms,
            repeat,
            cap,
            timeout,
            model,
            codex_version: String::from_utf8_lossy(&raw).trim().into(),
            endpoint: "task-complete".into(),
            retry_policy: "none".into(),
            git_history: "public-snapshot-only".into(),
        })
    }
}
pub fn validate_design(d: &Value) -> Result<()> {
    if !artifact::exact(
        d,
        &[
            "primary",
            "quality_margin",
            "minimum_tasks",
            "power_rationale",
            "selection",
            "exclusion",
            "secondary_benefit",
            "stopping_rule",
            "unknown_policy",
        ],
    ) || d["primary"] != "correctness"
        || d["stopping_rule"] != "fixed-horizon"
        || d["unknown_policy"] != "insufficient"
        || !d["minimum_tasks"].as_u64().is_some_and(|n| n > 0)
        || !d["quality_margin"]
            .as_f64()
            .is_some_and(|n| n.is_finite() && (0.0..1.0).contains(&n))
        || [
            "power_rationale",
            "selection",
            "exclusion",
            "secondary_benefit",
        ]
        .iter()
        .any(|k| !d[*k].as_str().is_some_and(|s| !s.trim().is_empty()))
    {
        return Err(artifact::invalid("invalid sealed correctness design"));
    }
    Ok(())
}
pub fn tasks(path: &Path) -> Result<Vec<Value>> {
    let document = artifact::read_json(path, artifact::MAX_TREE)?;
    let t = document["tasks"]
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or_else(|| artifact::invalid("completion task list required"))?;
    let mut ids = BTreeSet::new();
    for v in t {
        if !v["id"]
            .as_str()
            .is_some_and(|s| !s.is_empty() && ids.insert(s.to_string()))
            || !["answer", "patch"].contains(&v["task_mode"].as_str().unwrap_or(""))
            || !v["verifier_id"].is_string()
        {
            return Err(artifact::invalid("invalid or duplicate completion task"));
        }
    }
    Ok(t.clone())
}
pub fn cases(path: &Path, tasks: &[Value]) -> Result<Value> {
    let value = artifact::read_json(path, artifact::MAX_TREE)?;
    let c = value["cases"]
        .as_object()
        .ok_or_else(|| artifact::invalid("verifiers must contain cases"))?;
    for t in tasks {
        let case = c
            .get(t["verifier_id"].as_str().unwrap())
            .ok_or_else(|| artifact::invalid("missing verifier"))?;
        if case["mode"] != t["task_mode"] {
            return Err(artifact::invalid("verifier mode mismatch"));
        }
        if t.get("response_contract").is_some() {
            intake::configuration(t, case)?;
        } else if case.get("response").is_some() {
            return Err(artifact::invalid(
                "response verifier requires explicit public v2",
            ));
        }
    }
    Ok(Value::Object(c.clone()))
}
pub trait CorpusPreparation {
    /// Run with the held, explicitly selected native engine's installed-image authority.
    fn with_engine<T>(
        &self,
        operation: impl FnOnce(&executable::CanonicalEngine<'_>) -> Result<T>,
    ) -> Result<T>;
    fn build(&mut self, task: &Value, destination: &Path) -> Result<Value>;
    fn runtime_digest(&self, response: bool) -> Result<String>;
}
pub fn bindings(
    set: &Path,
    verifiers: &Path,
    cases: &Value,
    preparation: &impl CorpusPreparation,
) -> Result<Value> {
    let tasks = tasks(set)?;
    let mut receipts = json!({});
    for task in &tasks {
        if let Some(proof) = task.get("save_receipt") {
            let p = Path::new(
                proof["path"]
                    .as_str()
                    .ok_or_else(|| artifact::invalid("save receipt path"))?,
            );
            let hash = artifact::sha(&artifact::read_file(p, artifact::MAX_FILE)?.0);
            if proof["sha256"] != hash {
                return Err(artifact::invalid("captured receipt changed"));
            }
            receipts[task["id"].as_str().unwrap()] = hash.into();
        }
    }
    let mut artifacts = json!({});
    for (k, case) in cases
        .as_object()
        .ok_or_else(|| artifact::invalid("case map"))?
    {
        artifacts[k] = oracle::verifier_digest(case)?.into();
    }
    Ok(
        json!({"set_sha256":artifact::sha(&artifact::read_file(set,artifact::MAX_TREE)?.0),"save_receipts":receipts,"verifiers_sha256":artifact::sha(&artifact::read_file(verifiers,artifact::MAX_TREE)?.0),"verifier_artifacts":artifacts,"runtime_sha256":preparation.runtime_digest(tasks.iter().any(|t|t.get("response_contract").is_some()))?}),
    )
}
#[expect(
    clippy::too_many_arguments,
    reason = "The sealed protocol keeps source and verifier inputs, design, output, cache, settings and corpus preparation explicit."
)]
pub fn seal(
    set: &Path,
    verifiers: &Path,
    design: &Value,
    out: &Path,
    cache: &Path,
    settings: &Settings,
    preparation: &mut impl CorpusPreparation,
    source: &Path,
) -> Result<Value> {
    if out.exists() || artifact::overlaps(out, source)? || artifact::overlaps(cache, source)? {
        return Err(artifact::invalid(
            "fresh private protocol and external cache required",
        ));
    }
    validate_design(design)?;
    let tasks = tasks(set)?;
    let cases = cases(verifiers, &tasks)?;
    let prepared = artifact::Scratch::new(&cache.join("sealed"), "prepared")?;
    let persistent = prepared.0.clone();
    std::mem::forget(prepared);
    let mut bases = json!({});
    for t in &tasks {
        let id = t["id"].as_str().unwrap();
        let base = persistent.join(reconstruction::safe_name(id));
        let built = preparation.build(t, &base)?;
        if base.join(".git").exists() {
            std::fs::remove_dir_all(base.join(".git"))?;
        }
        let env = reconstruction::runtime_env(
            cache,
            t["ts"]
                .as_str()
                .and_then(reconstruction::instant)
                .ok_or_else(|| artifact::invalid("task timestamp"))?,
            false,
        )?;
        reconstruction::commit_corpus(&base, &env, None, None, &branch(t)?)?;
        let mut configs = json!({});
        for arm in &settings.arms {
            configs[arm] = config_digest(&base, arm)?.into();
        }
        let mut b = json!({"path":base,"sha256":artifact::candidate_digest(&base)?,"git_sha256":artifact::candidate_digest(&base.join(".git"))?,"configs":configs,"planted":built["planted"],"commit":built["commit"],"edition":built["edition"]});
        let mut variants = json!({});
        for arm in &settings.arms {
            if arm == "consolidate" || arm == "consolidate-verify" {
                let target = persistent.join(format!("{}-{arm}", reconstruction::safe_name(id)));
                let replay = preparation.with_engine(|engine| {
                    treatments::consolidation_with_engine(
                        source,
                        &base,
                        arm,
                        "local",
                        Some(&target),
                        engine,
                    )
                })?;
                if replay["validation_errors"]
                    .as_array()
                    .is_some_and(|e| !e.is_empty())
                {
                    return Err(artifact::invalid(
                        "consolidation canonical validation failed",
                    ));
                }
                configure(&target, arm)?;
                reconstruction::commit_corpus(&target, &env, None, None, &branch(t)?)?;
                variants[arm] = json!({"path":target,"sha256":artifact::candidate_digest(&target)?,"git_sha256":artifact::candidate_digest(&target.join(".git"))?,"replay":replay});
            }
        }
        if !variants.as_object().unwrap().is_empty() {
            b["variants"] = variants;
        }
        if t.get("response_contract").is_some() {
            b["response"] = intake::prepare(
                t,
                &cases[t["verifier_id"].as_str().unwrap()],
                &base,
                &persistent.join(format!("response-{}", reconstruction::safe_name(id))),
            )?;
        }
        bases[id] = b;
    }
    let mut p = json!({"schema":"task-outcome-protocol-v1","settings":settings,"design":design,"bindings":bindings(set,verifiers,&cases,preparation)?,"bases":bases});
    p["seal_sha256"] = artifact::digest(&p)?.into();
    artifact::write_json(out, &p)?;
    Ok(p)
}
pub fn validate_protocol(
    protocol: &Value,
    set: &Path,
    verifiers: &Path,
    cases: &Value,
    settings: &Settings,
    preparation: &impl CorpusPreparation,
) -> Result<()> {
    let mut body = protocol.clone();
    body.as_object_mut()
        .ok_or_else(|| artifact::invalid("protocol object required"))?
        .remove("seal_sha256");
    if protocol["schema"] != "task-outcome-protocol-v1"
        || protocol["seal_sha256"] != artifact::digest(&body)?
        || protocol["settings"] != json!(settings)
        || protocol["bindings"] != bindings(set, verifiers, cases, preparation)?
    {
        return Err(artifact::invalid("sealed input/runtime/verifier mismatch"));
    }
    validate_design(&protocol["design"])?;
    let tasks = tasks(set)?;
    let expected = tasks
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    let actual = protocol["bases"]
        .as_object()
        .ok_or_else(|| artifact::invalid("sealed bases required"))?
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if expected != actual {
        return Err(artifact::invalid("sealed task membership mismatch"));
    }
    for t in tasks {
        let b = &protocol["bases"][t["id"].as_str().unwrap()];
        let base = Path::new(
            b["path"]
                .as_str()
                .ok_or_else(|| artifact::invalid("sealed base path"))?,
        );
        if b["sha256"] != artifact::candidate_digest(base)?
            || b["git_sha256"] != artifact::candidate_digest(&base.join(".git"))?
        {
            return Err(artifact::invalid("sealed base changed"));
        }
        for arm in &settings.arms {
            if b["configs"][arm] != config_digest(base, arm)? {
                return Err(artifact::invalid("sealed config changed"));
            }
            if let Some(v) = b["variants"].get(arm) {
                let p = Path::new(
                    v["path"]
                        .as_str()
                        .ok_or_else(|| artifact::invalid("variant path"))?,
                );
                if v["sha256"] != artifact::candidate_digest(p)?
                    || v["git_sha256"] != artifact::candidate_digest(&p.join(".git"))?
                    || b["configs"][arm] != config_digest(p, arm)?
                {
                    return Err(artifact::invalid("sealed variant changed"));
                }
            }
        }
        if t.get("response_contract").is_some() {
            intake::validate(
                &t,
                &cases[t["verifier_id"].as_str().unwrap()],
                &b["response"],
                None,
            )?;
        }
    }
    Ok(())
}
pub fn branch(task: &Value) -> Result<String> {
    if let Some(s) = task["served_task_id"].as_str() {
        if s.len() > 151
            || s.is_empty()
            || !s
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "/_-".contains(c))
        {
            return Err(artifact::invalid("invalid served task branch"));
        }
        return Ok(s.into());
    }
    let mut name = format!(
        "ab/{}",
        reconstruction::safe_name(task["id"].as_str().unwrap_or(""))
    );
    if let Ok(re) = regex::Regex::new(r"[A-Za-z][A-Za-z0-9]*-[0-9]+")
        && let Some(ticket) = re.find(task["branch"].as_str().unwrap_or(""))
    {
        name.push('-');
        name.push_str(&ticket.as_str().to_uppercase());
    }
    Ok(name)
}
pub struct CompletionRun<'a> {
    pub set: &'a Path,
    pub verifiers: &'a Path,
    pub protocol: &'a Path,
    pub out: &'a Path,
    pub cache: &'a Path,
    pub private: &'a Path,
    pub source: &'a Path,
    pub codex: &'a str,
    pub executable: &'a Path,
    pub settings: &'a Settings,
}
pub fn run_complete(r: CompletionRun<'_>, preparation: &impl CorpusPreparation) -> Result<Value> {
    process::ensure_custody()?;
    if r.out.exists()
        || artifact::overlaps(r.out, r.source)?
        || artifact::overlaps(r.cache, r.private)?
    {
        return Err(artifact::invalid(
            "fresh external result and separated cache required",
        ));
    }
    let tasks = tasks(r.set)?;
    let cases = cases(r.verifiers, &tasks)?;
    let protocol = artifact::read_json(r.protocol, artifact::MAX_TREE)?;
    validate_protocol(
        &protocol,
        r.set,
        r.verifiers,
        &cases,
        r.settings,
        preparation,
    )?;
    let mut slots = vec![];
    for (index, t) in tasks.iter().enumerate() {
        for repeat in 0..r.settings.repeat {
            let mut arms = r.settings.arms.clone();
            if (index + repeat) % 2 == 1 {
                arms.reverse();
            }
            for arm in arms {
                slots.push(json!({"task_id":t["id"],"arm":arm,"repeat":repeat,"status":"pending","attempts":[]}));
            }
        }
    }
    let mut result = json!({"schema":"task-outcome-complete-v1","protocol":protocol,"meta":{"finished":false,"interrupted":false},"slots":slots});
    artifact::write_json(r.out, &result)?;
    let work = artifact::Scratch::new(r.cache, "completion")?;
    for ordinal in 0..result["slots"].as_array().unwrap().len() {
        process::ensure_custody()?;
        if crate::cancel::interrupted() {
            result["meta"]["interrupted"] = true.into();
            break;
        }
        let slot = result["slots"][ordinal].clone();
        let id = slot["task_id"].as_str().unwrap();
        let task = tasks.iter().find(|t| t["id"] == id).unwrap();
        let arm = slot["arm"].as_str().unwrap();
        let base = &protocol["bases"][id];
        let corpus = work.0.join(ordinal.to_string());
        let answer = work.0.join(format!("{ordinal}-answer.txt"));
        let citation = work.0.join(format!("{ordinal}-citations"));
        let mut attempt = json!({"ordinal":0,"completed":false,"usage":null,"verification":{"status":"unavailable","correct":null},"provider_attempt_coverage":"unknown","total_provider_tokens":null});
        result["slots"][ordinal]["status"] = "running".into();
        result["slots"][ordinal]["attempts"] = json!([attempt]);
        artifact::write_json(r.out, &result)?;
        let ran = (|| -> Result<()> {
            validate_protocol(
                &protocol,
                r.set,
                r.verifiers,
                &cases,
                r.settings,
                preparation,
            )?;
            let image = base["variants"].get(arm).unwrap_or(base);
            let original = Path::new(
                image["path"]
                    .as_str()
                    .ok_or_else(|| artifact::invalid("source base"))?,
            );
            preparation.with_engine(|engine| copy_corpus_with_engine(original, &corpus, engine))?;
            if artifact::candidate_digest(&corpus)? != image["sha256"] {
                return Err(artifact::invalid("arm source image mismatch"));
            }
            configure(&corpus, arm)?;
            if base["configs"][arm] != config_digest(&corpus, arm)? {
                return Err(artifact::invalid("arm configuration mismatch"));
            }
            let case = &cases[task["verifier_id"].as_str().unwrap()];
            let mut input = vec![];
            let mut prompt = task["prompt"]
                .as_str()
                .ok_or_else(|| artifact::invalid("task prompt"))?
                .to_string();
            let response = task.get("response_contract");
            if response.is_some() {
                copy_corpus(
                    Path::new(
                        base["response"]["path"]
                            .as_str()
                            .ok_or_else(|| artifact::invalid("citation source"))?,
                    ),
                    &citation,
                )?;
                let (_, raw) = intake::validate(task, case, &base["response"], Some(&citation))?;
                input = raw;
                prompt = "-".into();
                attempt["response_intake"] = base["response"].clone();
                attempt["response_intake"]
                    .as_object_mut()
                    .unwrap()
                    .remove("path");
            }
            let command = codex_command(
                r.codex,
                &corpus,
                arm,
                &prompt,
                Some(&r.settings.model),
                r.executable,
            )?;
            let view = isolation::prepare(
                &corpus,
                r.private,
                &command,
                true,
                response.map(|_| citation.as_path()),
            )?;
            if !isolation::probe(&view, r.verifiers) {
                return Err(artifact::invalid(
                    "private read isolation could not be proven",
                ));
            }
            let mut spec = view.spec();
            spec.input = input.clone();
            let gold = task["gold"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect::<Vec<_>>();
            let saved = base["planted"]["documents"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect::<Vec<_>>();
            let mut outcome = run_arm(ArmRequest {
                command: spec.argv,
                corpus: &corpus,
                gold: &gold,
                cap: r.settings.cap,
                timeout: Duration::from_secs_f64(r.settings.timeout),
                endpoint: "task-complete",
                env: spec.env,
                prompt_input: input,
                response_contract: response,
                answer: Some(&answer),
                raw_events: None,
                saved: &saved,
            })?;
            process::ensure_custody()?;
            for (k, v) in outcome.as_object().unwrap() {
                attempt[k] = v.clone();
            }
            attempt["isolation"] = isolation::BACKEND.into();
            attempt["runtime_sha256"] = protocol["bindings"]["runtime_sha256"].clone();
            attempt["config_sha256"] = base["configs"][arm].clone();
            result["slots"][ordinal]["attempts"] = json!([attempt.clone()]);
            artifact::write_json(r.out, &result)?;
            if arm == "selective" {
                let expected = json!({"router_digest":artifact::sha(include_bytes!("treatments.rs")),"rules_digest":artifact::digest(&treatments::rules())?,"sanitizer_digest":executable::digest(r.executable)?});
                let routing = artifact::read_file(
                    &corpus.join("memory-bank/local/experiments/route.jsonl"),
                    1048576,
                )
                .map(|(raw, _)| treatments::observations(&raw, &expected))
                .unwrap_or_else(
                    |_| json!({"status":"unknown","records":[],"producer_authenticated":false}),
                );
                outcome["routing"] = routing.clone();
                attempt["routing"] = routing;
            }
            attempt["treatment_valid"] = valid_arm(arm, &[outcome.clone()]).into();
            if outcome["completed"] == true {
                attempt["verification"] = if let Some(response) = response {
                    let context = json!({"protocol_sha256":protocol["seal_sha256"],"slot_id":format!("slot-{ordinal}"),"verifier_sha256":protocol["bindings"]["verifier_artifacts"][task["verifier_id"].as_str().unwrap()],"source_image_sha256":base["response"]["source_image_sha256"]});
                    attempt["response_artifact"] = response::make_artifact(
                        response,
                        &outcome["response_capture"]["messages"],
                        &context,
                    )?;
                    intake::grade(
                        r.executable,
                        &corpus,
                        &answer,
                        case,
                        task,
                        &base["response"],
                        &citation,
                        &context,
                        outcome["response_capture_sha256"].as_str(),
                    )
                } else {
                    oracle::verify(&corpus, &answer, case)
                };
                if attempt["verification"]["verifier_digest"]
                    != protocol["bindings"]["verifier_artifacts"]
                        [task["verifier_id"].as_str().unwrap()]
                {
                    attempt["verification"]["status"] = "unavailable".into();
                    attempt["verification"]["correct"] = Value::Null;
                }
            } else if ["cap", "timeout"].contains(&outcome["stopped"].as_str().unwrap_or("")) {
                attempt["verification"] = json!({"status":"not-completed","correct":false});
            } else {
                attempt["failure"] = "native-turn-or-cli-unavailable".into();
            }
            Ok(())
        })();
        if let Err(error) = &ran {
            process::record_custody_error(error);
        }
        if let Err(error) = process::ensure_custody() {
            result["meta"]["cleanup_unconfirmed"] = true.into();
            result["meta"]["failure_code"] = "cleanup-unconfirmed".into();
            let _ = artifact::write_json(r.out, &result);
            return Err(error);
        }
        if crate::cancel::interrupted() {
            result["slots"][ordinal]["status"] = "interrupted".into();
            result["meta"]["interrupted"] = true.into();
        } else if ran.is_ok() {
            result["slots"][ordinal]["status"] = "finished".into();
        } else {
            result["slots"][ordinal]["status"] = "unavailable".into();
            attempt["failure"] = "preflight-or-infrastructure-unavailable".into();
        }
        result["slots"][ordinal]["attempts"] = json!([attempt]);
        artifact::write_json(r.out, &result)?;
        process::ensure_custody()?;
        if corpus.exists() {
            std::fs::remove_dir_all(&corpus)?;
        }
        if citation.exists() {
            std::fs::remove_dir_all(&citation)?;
        }
        if answer.exists() {
            std::fs::remove_file(&answer)?;
        }
    }
    result["meta"]["finished"] = (!crate::cancel::interrupted()).into();
    artifact::write_json(r.out, &result)?;
    Ok(result)
}
pub fn copy_corpus(source: &Path, target: &Path) -> Result<()> {
    artifact::copy_native_tree(source, target, artifact::ImageKind::Physical, None)
}
pub(crate) fn copy_corpus_with_engine(
    source: &Path,
    target: &Path,
    engine: &executable::CanonicalEngine<'_>,
) -> Result<()> {
    artifact::copy_native_tree(source, target, artifact::ImageKind::Physical, Some(engine))
}
pub fn summarize_complete(result: &Value) -> Result<Value> {
    if result["schema"] != "task-outcome-complete-v1" {
        return Err(artifact::invalid(
            "completion report requires completion result",
        ));
    }
    let p = &result["protocol"];
    let mut body = p.clone();
    body.as_object_mut()
        .ok_or_else(|| artifact::invalid("invalid protocol"))?
        .remove("seal_sha256");
    if p["schema"] != "task-outcome-protocol-v1" || p["seal_sha256"] != artifact::digest(&body)? {
        return Err(artifact::invalid("completion report seal mismatch"));
    }
    validate_design(&p["design"])?;
    let s: Settings = serde_json::from_value(p["settings"].clone())?;
    if s.arms.len() != 2 || s.repeat == 0 || s.retry_policy != "none" {
        return Err(artifact::invalid("invalid completion settings"));
    }
    let tasks = p["bases"]
        .as_object()
        .ok_or_else(|| artifact::invalid("invalid bases"))?
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let expected = tasks
        .iter()
        .flat_map(|t| {
            s.arms
                .iter()
                .flat_map(move |a| (0..s.repeat).map(move |r| (t.clone(), a.clone(), r)))
        })
        .collect::<BTreeSet<_>>();
    let slots = result["slots"]
        .as_array()
        .ok_or_else(|| artifact::invalid("slots required"))?;
    let membership = slots
        .iter()
        .map(|slot| {
            Ok((
                slot["task_id"]
                    .as_str()
                    .ok_or_else(|| artifact::invalid("slot task"))?
                    .to_string(),
                slot["arm"]
                    .as_str()
                    .ok_or_else(|| artifact::invalid("slot arm"))?
                    .to_string(),
                slot["repeat"]
                    .as_u64()
                    .ok_or_else(|| artifact::invalid("slot repeat"))? as usize,
            ))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if slots.len() != expected.len() || membership != expected {
        return Err(artifact::invalid("lost or duplicated planned slots"));
    }
    let mut valid = result["meta"]["finished"] == true && result["meta"]["interrupted"] != true;
    let mut summaries = json!({});
    for arm in &s.arms {
        let chosen = slots
            .iter()
            .filter(|slot| slot["arm"] == *arm)
            .collect::<Vec<_>>();
        let attempts = chosen
            .iter()
            .flat_map(|slot| slot["attempts"].as_array().into_iter().flatten())
            .collect::<Vec<_>>();
        let last = chosen
            .iter()
            .map(|slot| {
                if slot["status"] == "finished" {
                    slot["attempts"]
                        .as_array()
                        .and_then(|a| a.last())
                        .cloned()
                        .unwrap_or(Value::Null)
                } else {
                    Value::Null
                }
            })
            .collect::<Vec<_>>();
        let correct = last
            .iter()
            .filter(|a| a["verification"]["correct"] == true)
            .count();
        let incorrect = last
            .iter()
            .filter(|a| a["verification"]["correct"] == false)
            .count();
        let unknown = chosen.len() - correct - incorrect;
        let mut summary = json!({"planned":chosen.len(),"attempted":chosen.iter().filter(|slot|slot["attempts"].as_array().is_some_and(|a|!a.is_empty())).count(),"unattempted":chosen.iter().filter(|slot|slot["attempts"].as_array().is_none_or(Vec::is_empty)).count(),"attempts":attempts.len(),"completed":last.iter().filter(|a|a["completed"]==true).count(),"verified":correct+incorrect,"correct":correct,"incorrect":incorrect,"unknown":unknown,"success_rate":if unknown==0{Some(correct as f64/chosen.len()as f64)}else{None},"provider_attempt_coverage":"unknown","total_provider_tokens":null,"total_usd":null,"cost_per_correct":null});
        let known = attempts.iter().map(|a| known_usage(a)).collect::<Vec<_>>();
        for (field, out) in [
            ("input_tokens", "known_input_tokens"),
            ("output_tokens", "known_output_tokens"),
            ("cached_input_tokens", "known_cached_input_tokens"),
        ] {
            summary[out] = json!(known.iter().filter_map(|u| u.get(field)).sum::<u64>());
        }
        summary["known_host_reported_tokens"] = json!(
            summary["known_input_tokens"].as_u64().unwrap()
                + summary["known_output_tokens"].as_u64().unwrap()
        );
        summary["attempts_with_known_usage"] = json!(
            known
                .iter()
                .filter(|u| u.contains_key("input_tokens") || u.contains_key("output_tokens"))
                .count()
        );
        summary["attempts_with_known_input_usage"] = json!(
            known
                .iter()
                .filter(|u| u.contains_key("input_tokens"))
                .count()
        );
        summary["attempts_with_known_output_usage"] = json!(
            known
                .iter()
                .filter(|u| u.contains_key("output_tokens"))
                .count()
        );
        summary["attempts_with_cached_usage"] = json!(
            known
                .iter()
                .filter(|u| u.contains_key("cached_input_tokens"))
                .count()
        );
        let usable = attempts
            .iter()
            .filter(|a| {
                a["usage_complete"] != false
                    && process::token(&a["usage"]["input_tokens"]).is_some()
                    && process::token(&a["usage"]["output_tokens"]).is_some()
            })
            .collect::<Vec<_>>();
        summary["attempts_with_usage"] = json!(usable.len());
        summary["host_reported_tokens"] = if chosen.iter().all(|slot| {
            slot["status"] == "finished"
                && slot["attempts"].as_array().is_some_and(|a| !a.is_empty())
        }) && usable.len() == attempts.len()
        {
            json!(
                usable
                    .iter()
                    .map(|a| a["usage"]["input_tokens"].as_u64().unwrap()
                        + a["usage"]["output_tokens"].as_u64().unwrap())
                    .sum::<u64>()
            )
        } else {
            Value::Null
        };
        let seconds = attempts
            .iter()
            .filter_map(|a| a["seconds"].as_f64())
            .filter(|n| n.is_finite() && *n >= 0.0)
            .collect::<Vec<_>>();
        let calls = attempts
            .iter()
            .filter_map(|a| a["calls"].as_u64())
            .collect::<Vec<_>>();
        summary["observed_agent_seconds"] = json!(seconds.iter().sum::<f64>());
        summary["attempts_with_seconds"] = json!(seconds.len());
        summary["observed_agent_calls"] = json!(calls.iter().sum::<u64>());
        summary["attempts_with_calls"] = json!(calls.len());
        summaries[arm] = summary;
    }
    let mut differences = vec![];
    for task in &tasks {
        let mut values = vec![];
        for arm in &s.arms {
            let chosen = slots
                .iter()
                .filter(|slot| slot["task_id"] == *task && slot["arm"] == *arm)
                .collect::<Vec<_>>();
            let mut correct = 0;
            for slot in &chosen {
                let attempts = slot["attempts"].as_array();
                valid &= attempts.is_some_and(|a| a.len() == 1) && slot["status"] == "finished";
                let a = attempts.and_then(|a| a.last()).unwrap_or(&Value::Null);
                valid &= a["verification"]["correct"].is_boolean() && a["treatment_valid"] == true;
                if a["verification"]["correct"] == true {
                    valid &= a["completed"] == true;
                    correct += 1;
                }
            }
            valid &= chosen.len() == s.repeat;
            values.push(correct as f64 / chosen.len().max(1) as f64);
        }
        differences.push(values[1] - values[0]);
    }
    valid &= tasks.len() >= p["design"]["minimum_tasks"].as_u64().unwrap() as usize;
    let interval = if valid && differences.len() >= 2 {
        rng::bootstrap(&differences, 10000, 20261021)
    } else {
        None
    };
    let verdict = if let Some(ci) = interval {
        if ci[0] > 0.0 {
            "treatment-helps"
        } else if ci[1] < -p["design"]["quality_margin"].as_f64().unwrap() {
            "treatment-hurts"
        } else {
            "no-difference-shown"
        }
    } else {
        "insufficient"
    };
    Ok(
        json!({"endpoint":"task-complete","arms":summaries,"tasks":tasks.len(),"paired_unit":"task","correctness_difference_ci95":interval,"verdict":verdict,"provider_cost_savings":null}),
    )
}
pub fn known_usage(attempt: &Value) -> BTreeMap<String, u64> {
    let turns = attempt["usage_coverage"]["completed_turns"].as_u64();
    if let Some(turns) = turns.filter(|n| *n > 0) {
        return process::USAGE_FIELDS
            .into_iter()
            .filter_map(|k| {
                process::token(&attempt["known_usage"][k])
                    .zip(attempt["usage_coverage"]["field_turns"][k].as_u64())
                    .filter(|(_, n)| *n > 0 && *n <= turns)
                    .map(|(v, _)| (k.into(), v))
            })
            .collect();
    }
    let u = &attempt["usage"];
    if process::token(&u["input_tokens"]).is_none() || process::token(&u["output_tokens"]).is_none()
    {
        return BTreeMap::new();
    }
    let mut out = process::USAGE_FIELDS
        .into_iter()
        .filter_map(|k| process::token(&u[k]).map(|v| (k.to_string(), v)))
        .collect::<BTreeMap<_, _>>();
    for (part, whole) in [
        ("cached_input_tokens", "input_tokens"),
        ("cache_write_input_tokens", "input_tokens"),
        ("reasoning_output_tokens", "output_tokens"),
    ] {
        if out
            .get(part)
            .zip(out.get(whole))
            .is_some_and(|(a, b)| a > b)
        {
            out.remove(part);
        }
    }
    out
}

pub fn summarize_legacy(result: &Value, compare: [&str; 2], metric: Option<&str>) -> Result<Value> {
    let metric = metric.unwrap_or_else(|| result["meta"]["metric"].as_str().unwrap_or("gold"));
    if !["gold", "recall"].contains(&metric) {
        return Err(artifact::invalid("metric is gold or recall"));
    }
    let cap = result["meta"]["cap"].as_u64().unwrap_or(40);
    let (baseline, treatment) = (compare[0], compare[1]);
    let mut pairs = vec![];
    let mut runs = BTreeMap::<String, Vec<Value>>::from([
        (baseline.into(), vec![]),
        (treatment.into(), vec![]),
    ]);
    for (id, item) in result["items"]
        .as_object()
        .ok_or_else(|| artifact::invalid("legacy result items required"))?
    {
        if item["status"] != "ok" {
            continue;
        }
        let get = |arm: &str| {
            item["arms"][arm].as_array().cloned().unwrap_or_else(|| {
                if item["arms"][arm].is_object() {
                    vec![item["arms"][arm].clone()]
                } else {
                    vec![]
                }
            })
        };
        let arms = [get(baseline), get(treatment)];
        if arms.iter().any(Vec::is_empty)
            || arms.iter().flatten().any(|r| r["stopped"] == "error")
            || !valid_arm(baseline, &arms[0])
            || !valid_arm(treatment, &arms[1])
        {
            continue;
        }
        let reached = |r: &Value| {
            r["calls_to_gold"].as_u64().or_else(|| {
                (metric == "recall")
                    .then(|| r["calls_to_answer"].as_u64())
                    .flatten()
            })
        };
        let mean = |rows: &[Value]| {
            rows.iter()
                .map(|r| reached(r).unwrap_or(cap + 1) as f64)
                .sum::<f64>()
                / rows.len() as f64
        };
        let mut pair = json!({"id":id});
        pair[baseline] = json!(mean(&arms[0]));
        pair[treatment] = json!(mean(&arms[1]));
        for (index, arm) in compare.iter().enumerate() {
            pair[format!("{arm}_touched")] =
                arms[index].iter().all(|r| reached(r).is_some()).into();
            pair[format!("{arm}_saved")] = arms[index]
                .iter()
                .any(|r| r["saved_delivered"] == true)
                .into();
            pair[format!("{arm}_ticket")] = arms[index]
                .iter()
                .any(|r| r["ticket_files_delivered"] == true)
                .into();
            runs.get_mut(*arm).unwrap().extend(arms[index].clone());
        }
        pairs.push(pair);
    }
    let differences = pairs
        .iter()
        .map(|p| p[treatment].as_f64().unwrap() - p[baseline].as_f64().unwrap())
        .collect::<Vec<_>>();
    let interval = if pairs.len() >= 2 {
        rng::bootstrap(&differences, 10000, 20261021)
    } else {
        None
    };
    let touched_b = pairs
        .iter()
        .filter(|p| p[format!("{baseline}_touched")] == true)
        .count();
    let touched_t = pairs
        .iter()
        .filter(|p| p[format!("{treatment}_touched")] == true)
        .count();
    let verdict = if let Some(ci) = interval.filter(|_| pairs.len() >= 20) {
        if ci[1] < 0.0 && touched_t + 1 >= touched_b {
            format!("{treatment}-helps")
        } else if ci[0] > 0.0 {
            format!("{treatment}-hurts")
        } else {
            "no-difference-shown".into()
        }
    } else {
        "insufficient".into()
    };
    let mut summary = json!({"compare":compare,"metric":metric,"tasks":result["items"].as_object().map(|m|m.len()).unwrap_or(0),"pairs":pairs.len(),"cap":cap,"mean_difference":if differences.is_empty(){None}else{Some(differences.iter().sum::<f64>()/differences.len()as f64)},"ci95":interval,"sign_test_p":if differences.is_empty(){None}else{Some(rng::sign_test(&differences))},"verdict":verdict});
    for arm in compare {
        summary[format!("mean_{arm}")] = if pairs.is_empty() {
            Value::Null
        } else {
            json!(pairs.iter().filter_map(|p| p[arm].as_f64()).sum::<f64>() / pairs.len() as f64)
        };
        summary[format!("touched_{arm}")] = json!(
            pairs
                .iter()
                .filter(|p| p[format!("{arm}_touched")] == true)
                .count()
        );
        if metric == "recall" {
            summary[format!("saved_delivered_{arm}")] = json!(
                pairs
                    .iter()
                    .filter(|p| p[format!("{arm}_saved")] == true)
                    .count()
            );
        }
    }
    summary[format!("{treatment}_fewer")] = json!(differences.iter().filter(|n| **n < 0.0).count());
    summary[format!("{treatment}_more")] = json!(differences.iter().filter(|n| **n > 0.0).count());
    if treatment == "ticket" {
        summary["ticket_files_delivered"] = json!(
            pairs
                .iter()
                .filter(|p| p[format!("{treatment}_ticket")] == true)
                .count()
        );
    }
    let mut costs = json!({});
    for arm in compare {
        let rows = &runs[arm];
        let mean = |key: &str| {
            let v = rows
                .iter()
                .filter_map(|r| r[key].as_f64())
                .collect::<Vec<_>>();
            (!v.is_empty()).then(|| (v.iter().sum::<f64>() / v.len() as f64 * 10.0).round() / 10.0)
        };
        let mut cost = json!({"runs":rows.len(),"seconds_mean":mean("seconds"),"runs_with_usage":rows.iter().filter(|r|r["usage"].is_object()).count(),"capsule_tokens_mean":mean("capsule_tokens"),"capsule_bytes_mean":mean("capsule_bytes")});
        for key in ["input_tokens", "cached_input_tokens", "output_tokens"] {
            let v = rows
                .iter()
                .filter_map(|r| r["usage"][key].as_f64())
                .collect::<Vec<_>>();
            cost[format!("{key}_mean")] = if v.is_empty() {
                Value::Null
            } else {
                json!((v.iter().sum::<f64>() / v.len() as f64 * 10.0).round() / 10.0)
            };
        }
        costs[arm] = cost;
    }
    summary["cost"] = costs;
    Ok(summary)
}
#[expect(
    clippy::too_many_arguments,
    reason = "The legacy experiment entry point preserves independent arm, repetition, budget, timeout, model, event and corpus controls."
)]
pub fn run_legacy(
    set: &Value,
    out: &Path,
    arms: &[String],
    repeat: usize,
    cap: usize,
    timeout: Duration,
    model: Option<&str>,
    codex: &str,
    events: Option<&Path>,
    keep: bool,
    preparation: &mut crate::sampling::NativePreparation,
) -> Result<Value> {
    process::ensure_custody()?;
    if arms.is_empty()
        || arms.iter().any(|a| {
            !ARMS.contains(&a.as_str())
                || ["selective", "consolidate", "consolidate-verify"].contains(&a.as_str())
        })
        || repeat == 0
        || cap == 0
    {
        return Err(artifact::invalid("invalid first-touch arms or budgets"));
    }
    if artifact::overlaps(out, &preparation.reconstruction.source)?
        || events.is_some_and(|p| {
            artifact::overlaps(p, &preparation.reconstruction.source).unwrap_or(true)
        })
    {
        return Err(artifact::invalid(
            "private results and events outside tooling clone required",
        ));
    }
    let tasks = set["tasks"]
        .as_array()
        .ok_or_else(|| artifact::invalid("sample task list required"))?;
    let ids = tasks
        .iter()
        .filter_map(|t| t["id"].as_str())
        .collect::<BTreeSet<_>>();
    if ids.len() != tasks.len() {
        return Err(artifact::invalid("invalid or duplicate task ID"));
    }
    let work = artifact::Scratch::new(&preparation.reconstruction.cache, "outcome")?;
    let mut result = json!({"version":1,"meta":{"started":reconstruction::now(),"arms":arms,"cap":cap,"timeout":timeout.as_secs_f64(),"model":model,"repeat":repeat,"kind":if set["kind"]=="recall"{"recall"}else{"cold-start"},"metric":if set["kind"]=="recall"{"recall"}else{"gold"}},"items":{}});
    for (index, t) in tasks.iter().enumerate() {
        process::ensure_custody()?;
        if crate::cancel::interrupted() {
            result["meta"]["interrupted"] = true.into();
            break;
        }
        let id = t["id"].as_str().unwrap();
        let base = work
            .0
            .join(format!("{}-base", reconstruction::safe_name(id)));
        let mut item = json!({"project":t["project"],"ts":t["ts"],"gold":t["gold"].as_array().map(Vec::len).unwrap_or(0),"arms":{}});
        let run = (|| -> Result<()> {
            let built = preparation.build(t, &base)?;
            item["commit"] = built["commit"].clone();
            item["edition"] = built["edition"].clone();
            item["recent_files"] = built["recent_files"].clone();
            item["planted"] = built["planted"].clone();
            let gold = t["gold"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect::<Vec<_>>();
            let saved = built["planted"]["documents"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect::<Vec<_>>();
            for replicate in 0..repeat {
                let mut order = arms.to_vec();
                if (index + replicate) % 2 == 1 {
                    order.reverse();
                }
                for arm in order {
                    process::ensure_custody()?;
                    let corpus = work.0.join(format!(
                        "{}-{arm}-{replicate}",
                        reconstruction::safe_name(id)
                    ));
                    preparation
                        .with_engine(|engine| copy_corpus_with_engine(&base, &corpus, engine))?;
                    configure(&corpus, &arm)?;
                    let command = codex_command(
                        codex,
                        &corpus,
                        &arm,
                        t["prompt"]
                            .as_str()
                            .ok_or_else(|| artifact::invalid("task prompt"))?,
                        model,
                        &preparation.reconstruction.runtime,
                    )?;
                    let event_path =
                        events.map(|dir| dir.join(format!("{id}-{arm}-{replicate}.jsonl")));
                    let outcome = run_arm(ArmRequest {
                        command,
                        corpus: &corpus,
                        gold: &gold,
                        cap,
                        timeout,
                        endpoint: "first-touch",
                        env: reconstruction::clean_env(),
                        prompt_input: vec![],
                        response_contract: None,
                        answer: None,
                        raw_events: event_path.as_deref(),
                        saved: &saved,
                    })?;
                    process::ensure_custody()?;
                    if item["arms"].get(&arm).is_none() {
                        item["arms"][&arm] = json!([]);
                    }
                    item["arms"][&arm].as_array_mut().unwrap().push(outcome);
                    if !keep {
                        std::fs::remove_dir_all(&corpus)?;
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = &run {
            process::record_custody_error(error);
        }
        if let Err(error) = process::ensure_custody() {
            result["meta"]["cleanup_unconfirmed"] = true.into();
            result["meta"]["failure_code"] = "cleanup-unconfirmed".into();
            let _ = artifact::write_json(out, &result);
            return Err(error);
        }
        item["status"] = if run.is_ok() {
            "ok".into()
        } else {
            "skipped:corpus-or-runtime-unavailable".into()
        };
        result["items"][id] = item;
        artifact::write_json(out, &result)?;
        process::ensure_custody()?;
        if !keep && base.exists() {
            std::fs::remove_dir_all(&base)?;
        }
    }
    if !crate::cancel::interrupted() {
        result["meta"]["finished"] = reconstruction::now().into();
    }
    artifact::write_json(out, &result)?;
    if keep {
        let path = work.0.clone();
        std::mem::forget(work);
        result["meta"]["kept_work"] = json!(path);
    }
    Ok(result)
}

#[cfg(test)]
mod engine_copy_tests {
    use super::*;
    use std::io::{Seek, SeekFrom, Write};

    #[test]
    fn canonical_engine_tree_copy_keeps_data_limits_and_native_directory_metadata() {
        let work = artifact::Scratch::new(&std::env::temp_dir(), "outcome-engine-copy").unwrap();
        let source = work.0.join("source");
        artifact::private_dir(&source).unwrap();
        let slot = if cfg!(windows) {
            "memory-bank/bin/infra-engine.exe"
        } else {
            "memory-bank/bin/infra-engine"
        };
        // Byte-admission fixture only: never execute this structurally valid ELF.
        let mut bytes = vec![0; artifact::MAX_TREE + 1];
        bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        bytes[16..18].copy_from_slice(&2u16.to_le_bytes());
        bytes[18..20].copy_from_slice(&62u16.to_le_bytes());
        bytes[20..24].copy_from_slice(&1u32.to_le_bytes());
        bytes[52..54].copy_from_slice(&64u16.to_le_bytes());
        artifact::fresh_file(&source.join(slot), &bytes, 0o755).unwrap();
        std::fs::create_dir_all(source.join("empty/leaf")).unwrap();
        artifact::chmod(&source.join("empty"), 0o750).unwrap();
        artifact::chmod(&source.join("empty/leaf"), 0o710).unwrap();
        artifact::chmod(&source, 0o750).unwrap();
        let engine = executable::CanonicalEngine::new(slot, &bytes, 0o755).unwrap();
        let generic = work.0.join("generic-refused");
        assert!(copy_corpus(&source, &generic).is_err());
        assert!(
            !generic.exists(),
            "generic tree-copy data limits must remain unchanged"
        );
        let target = work.0.join("admitted");
        copy_corpus_with_engine(&source, &target, &engine).unwrap();
        assert_eq!(
            artifact::sha(
                &artifact::read_file(&target.join(slot), executable::LIMIT)
                    .unwrap()
                    .0
            ),
            artifact::sha(&bytes)
        );
        assert!(target.join("empty/leaf").is_dir());
        #[cfg(unix)]
        for (name, mode) in [("", 0o750), ("empty", 0o750), ("empty/leaf", 0o710)] {
            assert_eq!(artifact::mode(&target.join(name)).unwrap(), mode);
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(source.join(slot))
            .unwrap();
        file.seek(SeekFrom::End(-1)).unwrap();
        file.write_all(&[1]).unwrap();
        drop(file);
        let refused = work.0.join("modified-refused");
        assert!(copy_corpus_with_engine(&source, &refused, &engine).is_err());
        assert!(
            !refused.exists(),
            "changed engine must fail before target publication"
        );
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(source.join(slot))
            .unwrap();
        file.seek(SeekFrom::End(-1)).unwrap();
        file.write_all(&[0]).unwrap();
        drop(file);
        std::fs::File::create(source.join("oversized-data.bin"))
            .unwrap()
            .set_len(artifact::MAX_FILE as u64 + 1)
            .unwrap();
        let refused = work.0.join("data-refused");
        assert!(copy_corpus_with_engine(&source, &refused, &engine).is_err());
        assert!(
            !refused.exists(),
            "ordinary files retain their independent 4 MiB limit"
        );
    }
}
